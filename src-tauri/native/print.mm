// SPDX-License-Identifier: GPL-3.0-or-later
// macOS printing of a validated, leased article PDF.
#import <Foundation/Foundation.h>
#import <AppKit/AppKit.h>
#import <PDFKit/PDFKit.h>
#import <ApplicationServices/ApplicationServices.h>
#include <cstdint>

using PdfAuthority = bool (*)(uint64_t);
using PdfCompletion = void (*)(uint64_t, uint8_t, int32_t);

// These codes match print_complete in src/native_pdf/apple.rs.
enum class PdfPrintStatus : uint8_t {
    Success = 0,
    AuthorityExpired = 1,
    InvalidDocument = 2,
    OperationUnavailable = 3,
    WindowUnavailable = 4,
    WindowBusy = 5,
    Cancelled = 6,
    TimedOut = 7,
    SystemError = 8,
    OperationBusy = 10,
};

static constexpr NSTimeInterval KiwixPdfPrintPanelLimit = 15 * 60;

@interface KiwixPdfPrintRequest : NSObject
@property(nonatomic) uint64_t token;
@property(nonatomic) PdfAuthority authority;
@property(nonatomic) PdfCompletion completion;
@property(nonatomic, strong) NSURL *url;
@property(nonatomic, strong) NSString *title;
@property(nonatomic, strong) PDFDocument *document;
@property(nonatomic, strong) NSPrintOperation *operation;
@property(nonatomic, strong) NSWindow *host;
@property(nonatomic, strong) NSTimer *timer;
@property(nonatomic) PdfPrintStatus cancellation;
@property(nonatomic) BOOL finished;
@property(nonatomic) BOOL operationStarted;
@property(nonatomic) BOOL operationActive;
- (void)begin;
- (void)cancel:(PdfPrintStatus)reason;
- (void)finish:(PdfPrintStatus)status systemError:(OSStatus)systemError;
- (void)printOperationDidRun:(NSPrintOperation *)operation success:(BOOL)success contextInfo:(void *)contextInfo;
@end

static NSMutableDictionary<NSNumber *, KiwixPdfPrintRequest *> *activePrintRequests() {
    static NSMutableDictionary<NSNumber *, KiwixPdfPrintRequest *> *requests = [NSMutableDictionary new];
    return requests;
}

static void clearUnstartedOperation(NSPrintOperation *operation) {
    if (operation && NSPrintOperation.currentOperation == operation) {
        NSPrintOperation.currentOperation = nil;
    }
}

@implementation KiwixPdfPrintRequest
- (PdfPrintStatus)hostStatus {
    NSWindow *host = self.host;
    NSWindow *selected = host.tabGroup.selectedWindow;
    if (!host.isVisible || ![NSApp.windows containsObject:host] || (selected && selected != host)) {
        return PdfPrintStatus::WindowUnavailable;
    }
    if (host.sheets.count) return PdfPrintStatus::WindowBusy;
    for (KiwixPdfPrintRequest *request in activePrintRequests().allValues) {
        if (request != self && request.host == host) return PdfPrintStatus::WindowBusy;
    }
    return PdfPrintStatus::Success;
}

- (void)begin {
    if (self.finished) return;
    PdfPrintStatus windowStatus = [self hostStatus];
    if (windowStatus != PdfPrintStatus::Success) {
        [self finish:windowStatus systemError:noErr];
        return;
    }
    PDFDocument *document = [[PDFDocument alloc] initWithURL:self.url];
    if (!document || document.pageCount <= 0) {
        [self finish:PdfPrintStatus::InvalidDocument systemError:noErr];
        return;
    }
    if (NSPrintOperation.currentOperation) {
        [self finish:PdfPrintStatus::OperationBusy systemError:noErr];
        return;
    }
    NSPrintOperation *operation = nil;
    @try {
        operation = [document printOperationForPrintInfo:[NSPrintInfo sharedPrintInfo]
            scalingMode:kPDFPrintPageScaleToFit autoRotate:YES];
    } @catch (NSException *exception) {
        PdfPrintStatus status = [exception.name isEqualToString:NSPrintOperationExistsException]
            ? PdfPrintStatus::OperationBusy : PdfPrintStatus::OperationUnavailable;
        [self finish:status systemError:noErr];
        return;
    }
    if (!operation) {
        [self finish:PdfPrintStatus::OperationUnavailable systemError:noErr];
        return;
    }
    operation.jobTitle = self.title;
    operation.showsPrintPanel = YES;
    operation.showsProgressPanel = YES;
    // A previous Save-as-PDF disposition must never skip printer selection.
    operation.printInfo.jobDisposition = NSPrintSpoolJob;

    if (self.finished) {
        clearUnstartedOperation(operation);
        return;
    }
    self.document = document;
    self.operation = operation;
    windowStatus = [self hostStatus];
    if (windowStatus != PdfPrintStatus::Success) {
        [self finish:windowStatus systemError:noErr];
        return;
    }
    __weak KiwixPdfPrintRequest *weakSelf = self;
    self.timer = [NSTimer timerWithTimeInterval:KiwixPdfPrintPanelLimit repeats:NO block:^(NSTimer *) {
        [weakSelf cancel:PdfPrintStatus::TimedOut];
    }];
    [[NSRunLoop mainRunLoop] addTimer:self.timer forMode:NSRunLoopCommonModes];

    // Recheck guarded application authority after PDFKit prepares the job.
    // AppKit's modal entry can call back into the host event loop, so invoke it
    // only after the Rust callback has returned and released its guards.
    if (!self.authority || !self.authority(self.token)) {
        [self finish:PdfPrintStatus::AuthorityExpired systemError:noErr];
        return;
    }
    self.operationStarted = YES;
    self.operationActive = YES;
    [operation runOperationModalForWindow:self.host delegate:self
        didRunSelector:@selector(printOperationDidRun:success:contextInfo:) contextInfo:nullptr];
}

- (void)cancel:(PdfPrintStatus)reason {
    if (self.finished || self.cancellation != PdfPrintStatus::Success) return;
    self.cancellation = reason;
    [self.timer invalidate]; self.timer = nil;
    if (!self.operationActive) { [self finish:reason systemError:noErr]; return; }
    // Cancel only the operation's own Core Printing session. An attached
    // sheet can belong to a different action. Keep the document, delegate
    // and scratch lease until this operation reports its actual completion.
    PMPrintSession session = (PMPrintSession)self.operation.printInfo.PMPrintSession;
    if (session) PMSessionSetError(session, kPMCancel);
}

- (void)finish:(PdfPrintStatus)status systemError:(OSStatus)systemError {
    if (self.finished || self.operationActive) return;
    self.finished = YES;
    [self.timer invalidate]; self.timer = nil;
    if (!self.operationStarted) clearUnstartedOperation(self.operation);
    self.document = nil; self.operation = nil; self.host = nil;
    uint64_t token = self.token;
    PdfCompletion callback = self.completion;
    self.completion = nullptr; self.authority = nullptr;
    [activePrintRequests() removeObjectForKey:@(token)];
    if (callback) callback(token, static_cast<uint8_t>(status), systemError);
}

- (void)printOperationDidRun:(NSPrintOperation *)operation success:(BOOL)success contextInfo:(void *)contextInfo {
    // AppKit can call this delegate from its printing thread. Keep the delegate
    // alive past the end of AppKit's callback stack.
    dispatch_async(dispatch_get_main_queue(), ^{
        if (self.finished || !self.operationActive || operation != self.operation) return;
        self.operationActive = NO;
        if (self.cancellation != PdfPrintStatus::Success) {
            [self finish:self.cancellation systemError:noErr];
            return;
        }
        if (!success) {
            PMPrintSession session = (PMPrintSession)operation.printInfo.PMPrintSession;
            OSStatus status = session ? PMSessionError(session) : kPMInvalidPrintSession;
            // Closing the dialog is a normal completion.
            if (status != noErr && status != kPMCancel) {
                [self finish:PdfPrintStatus::SystemError systemError:status];
                return;
            }
        }
        [self finish:PdfPrintStatus::Success systemError:noErr];
    });
}
@end

extern "C" bool kiwix_pdf_print_start(uint64_t token, void *rawHost, const char *path, const char *title,
                                      PdfAuthority authority, PdfCompletion complete) {
    if (!NSThread.isMainThread || !rawHost || !path || !title || !authority || !complete
        || activePrintRequests()[@(token)]) return false;
    NSString *pathString = [NSString stringWithUTF8String:path];
    NSString *titleString = [NSString stringWithUTF8String:title];
    if (!pathString || !titleString) return false;
    KiwixPdfPrintRequest *request = [KiwixPdfPrintRequest new];
    request.token = token;
    request.host = (__bridge NSWindow *)rawHost;
    request.authority = authority;
    request.completion = complete;
    request.url = [NSURL fileURLWithPath:pathString];
    request.title = titleString;
    activePrintRequests()[@(token)] = request;
    [request begin];
    return true;
}

extern "C" void kiwix_pdf_print_cancel(uint64_t token) {
    dispatch_async(dispatch_get_main_queue(), ^{ [activePrintRequests()[@(token)] cancel:PdfPrintStatus::Cancelled]; });
}
