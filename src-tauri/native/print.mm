// SPDX-License-Identifier: GPL-3.0-or-later
//! Native macOS print handoff for an already-rendered article PDF.
//!
//! The PDF renderer in `pdf.mm` writes a private temporary file under the same
//! admission/commit gate as Save. This file only presents that validated file
//! through the real system print panel (PDFKit's `NSPrintOperation`), so the
//! caller can keep the temp file until the panel closes and remove it after.
#import <Foundation/Foundation.h>
#import <AppKit/AppKit.h>
#import <PDFKit/PDFKit.h>
#import <ApplicationServices/ApplicationServices.h>
#include <cstdint>

using PdfAuthority = bool (*)(void *);
using PdfCompletion = void (*)(void *, const char *);

// The panel is user-driven, so the bound is generous, but it is finite: an
// abandoned sheet must not retain archive bytes or the temporary PDF forever.
static constexpr NSTimeInterval KiwixPdfPrintPanelLimit = 15 * 60;
// Cooperative Core Printing cancellation is not a completion signal. If the
// sheet does not report back shortly after we end it, finish the request so the
// Rust owner can reclaim the file.
static constexpr NSTimeInterval KiwixPdfPrintCancelSettleLimit = 5;

@interface KiwixPdfPrintRequest : NSObject
@property(nonatomic) uint64_t token;
@property(nonatomic) void *context;
@property(nonatomic) PdfAuthority authority;
@property(nonatomic) PdfCompletion completion;
@property(nonatomic, strong) NSURL *url;
@property(nonatomic, strong) NSString *title;
@property(nonatomic, strong) PDFDocument *document;
@property(nonatomic, strong) NSPrintOperation *operation;
@property(nonatomic, strong) NSWindow *host;
@property(nonatomic, strong) NSTimer *timer;
@property(nonatomic, strong) NSTimer *settleTimer;
@property(nonatomic, strong) NSString *failure;
@property(nonatomic) BOOL finished;
@property(nonatomic) BOOL operationActive;
- (void)begin;
- (void)cancel:(NSString *)error;
- (void)finish:(NSString *)error;
- (void)printOperationDidRun:(NSPrintOperation *)operation success:(BOOL)success contextInfo:(void *)contextInfo;
@end

static NSMutableDictionary<NSNumber *, KiwixPdfPrintRequest *> *printRequests;
static NSMutableDictionary<NSNumber *, KiwixPdfPrintRequest *> *activePrintRequests(void) {
    if (!printRequests) printRequests = [NSMutableDictionary new];
    return printRequests;
}

@implementation KiwixPdfPrintRequest
- (void)begin {
    if (self.finished) return;
    if (!self.authority || !self.authority(self.context)) {
        [self finish:@"Article PDF print authority expired"];
        return;
    }
    PDFDocument *document = [[PDFDocument alloc] initWithURL:self.url];
    if (!document || document.pageCount <= 0) {
        [self finish:@"Cannot open the rendered article PDF for printing"];
        return;
    }
    NSPrintOperation *operation = [document printOperationForPrintInfo:[NSPrintInfo sharedPrintInfo]
        scalingMode:kPDFPrintPageScaleToFit autoRotate:YES];
    if (!operation) {
        [self finish:@"Cannot create the native print operation"];
        return;
    }
    operation.jobTitle = self.title;
    operation.showsPrintPanel = YES;
    operation.showsProgressPanel = YES;
    // Keep the user's page range/copies/settings meaningful without allowing a
    // prior Save-as-PDF disposition to bypass the printer selection panel.
    operation.printInfo.jobDisposition = NSPrintSpoolJob;
    self.document = document;
    self.operation = operation;
    NSWindow *host = NSApp.mainWindow ?: NSApp.keyWindow;
    if (!host) {
        for (NSWindow *window in NSApp.windows) {
            if (window.isVisible && window.canBecomeMainWindow) { host = window; break; }
        }
    }
    if (!host) {
        [self finish:@"No application window is available for the print panel"];
        return;
    }
    self.host = host;
    self.operationActive = YES;
    __weak KiwixPdfPrintRequest *weakSelf = self;
    self.timer = [NSTimer timerWithTimeInterval:KiwixPdfPrintPanelLimit repeats:NO block:^(NSTimer *) {
        [weakSelf cancel:@"The system print dialog timed out"];
    }];
    [[NSRunLoop mainRunLoop] addTimer:self.timer forMode:NSRunLoopCommonModes];
    NSLog(@"Kiwix print: presenting %@ (%ld pages)", self.title, (long)document.pageCount);
    [operation runOperationModalForWindow:host delegate:self
        didRunSelector:@selector(printOperationDidRun:success:contextInfo:) contextInfo:nullptr];
}

- (void)cancel:(NSString *)error {
    if (self.finished || self.failure) return;
    self.failure = error;
    [self.timer invalidate]; self.timer = nil;
    if (!self.operationActive) { [self finish:error]; return; }
    // Core Printing cancellation is cooperative. End the visible sheet as well
    // so a policy change or shutdown does not leave it open indefinitely.
    PMPrintSession session = (PMPrintSession)self.operation.printInfo.PMPrintSession;
    if (session) PMSessionSetError(session, kPMCancel);
    NSWindow *sheet = self.host.attachedSheet;
    if (sheet) [self.host endSheet:sheet returnCode:NSModalResponseCancel];
    __weak KiwixPdfPrintRequest *weakSelf = self;
    self.settleTimer = [NSTimer timerWithTimeInterval:KiwixPdfPrintCancelSettleLimit repeats:NO block:^(NSTimer *) {
        KiwixPdfPrintRequest *request = weakSelf;
        if (!request || request.finished) return;
        request.operationActive = NO;
        [request finish:request.failure ?: @"The native print operation did not finish after cancellation"];
    }];
    [[NSRunLoop mainRunLoop] addTimer:self.settleTimer forMode:NSRunLoopCommonModes];
}

- (void)finish:(NSString *)error {
    if (self.finished) return;
    if (self.operationActive) {
        if (error) [self cancel:error];
        // Cancellation is not completion. Retain the document, panel and
        // admission ownership until the native operation settles.
        return;
    }
    self.finished = YES;
    [self.timer invalidate]; self.timer = nil;
    [self.settleTimer invalidate]; self.settleTimer = nil;
    self.document = nil; self.operation = nil; self.host = nil;
    PdfCompletion callback = self.completion;
    void *context = self.context;
    self.completion = nullptr; self.authority = nullptr; self.context = nullptr;
    [activePrintRequests() removeObjectForKey:@(self.token)];
    if (callback) callback(context, error ? error.UTF8String : nullptr);
}

- (void)printOperationDidRun:(NSPrintOperation *)operation success:(BOOL)success contextInfo:(void *)contextInfo {
    // AppKit may invoke the delegate on its printing thread.
    dispatch_async(dispatch_get_main_queue(), ^{
        if (self.finished || operation != self.operation) return;
        self.operationActive = NO;
        [self.settleTimer invalidate]; self.settleTimer = nil;
        if (self.failure) { [self finish:self.failure]; return; }
        if (!success) {
            OSStatus status = PMSessionError((PMPrintSession)operation.printInfo.PMPrintSession);
            // The user closing the panel is a normal completion; only report a
            // real Core Printing failure.
            if (status != noErr && status != kPMCancel) {
                [self finish:[NSString stringWithFormat:@"The native print operation failed (%d)", (int)status]];
                return;
            }
        }
        [self finish:nil];
    });
}
@end

extern "C" bool kiwix_pdf_print_start(uint64_t token, const char *path, const char *title, void *context, PdfAuthority authority, PdfCompletion complete) {
    NSString *pathString = path ? [NSString stringWithUTF8String:path] : nil;
    NSString *titleString = title ? [NSString stringWithUTF8String:title] : nil;
    if (!pathString || !titleString || !complete) return false;
    dispatch_async(dispatch_get_main_queue(), ^{
        KiwixPdfPrintRequest *request = [KiwixPdfPrintRequest new];
        request.token = token; request.context = context;
        request.authority = authority; request.completion = complete;
        request.url = [NSURL fileURLWithPath:pathString];
        request.title = titleString;
        activePrintRequests()[@(token)] = request;
        [request begin];
    });
    return true;
}

extern "C" void kiwix_pdf_print_cancel(uint64_t token) {
    dispatch_async(dispatch_get_main_queue(), ^{
        [activePrintRequests()[@(token)] cancel:@"Article PDF printing was cancelled"];
    });
}
