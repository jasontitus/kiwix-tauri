// SPDX-License-Identifier: GPL-3.0-or-later
#import <Foundation/Foundation.h>
#import <WebKit/WebKit.h>
#import <TargetConditionals.h>
#import <CoreGraphics/CoreGraphics.h>
#if TARGET_OS_IPHONE
#import <UIKit/UIKit.h>
#else
#import <AppKit/AppKit.h>
#endif
#include <cstdint>
#include <cmath>
#include <algorithm>
using PdfAuthority = bool (*)(uint64_t);
using PdfCompletion = void (*)(uint64_t, uint8_t);
// These ABI values match render_result in src/native_pdf/apple.rs.
enum class PdfRenderStatus : uint8_t {
    Success = 0,
    NavigationFailed = 1,
    RendererTerminated = 2,
    AuthorityExpired = 3,
    PageLimit = 4,
    OutputUnavailable = 5,
    PlatformUnavailable = 6,
    PageCaptureFailed = 7,
    InvalidPage = 8,
    InvalidPageDimensions = 9,
    OutputLimit = 10,
    InvalidCaptureDimensions = 11,
    TimedOut = 12,
    Cancelled = 13,
};
static constexpr unsigned long long KiwixPdfByteLimit = 64ULL * 1024 * 1024;
static constexpr double KiwixPaperWidth = 595.28;
static constexpr double KiwixPaperHeight = 841.89;

@interface KiwixPdfRequest : NSObject <WKNavigationDelegate>
@property(nonatomic) uint64_t token;
@property(nonatomic) PdfAuthority authority;
@property(nonatomic) PdfCompletion completion;
@property(nonatomic, strong) WKWebView *view;
@property(nonatomic, strong) NSURL *bootstrap;
@property(nonatomic, strong) NSURL *article;
@property(nonatomic, strong) NSURL *output;
@property(nonatomic, strong) NSTimer *timer;
@property(nonatomic) BOOL finished;
@property(nonatomic) BOOL rendering;
@property(nonatomic) double sourceWidth;
@property(nonatomic) double sourceHeight;
@property(nonatomic) double sourcePageHeight;
@property(nonatomic) NSUInteger nextPage;
@property(nonatomic) NSUInteger pageCount;
@property(nonatomic) CGContextRef pdfContext;
- (void)finish:(PdfRenderStatus)status;
- (void)capturePage;
@end
static NSMutableDictionary<NSNumber *, KiwixPdfRequest *> *pdfRequests;
static NSMutableDictionary<NSNumber *, KiwixPdfRequest *> *activePdfRequests() {
    if (!pdfRequests) pdfRequests = [NSMutableDictionary new];
    return pdfRequests;
}
@implementation KiwixPdfRequest
- (void)finish:(PdfRenderStatus)status {
    if (self.finished) return;
    self.finished = YES;
    [self.timer invalidate]; self.timer = nil;
    [self.view stopLoading]; self.view.navigationDelegate = nil;
    [self.view removeFromSuperview]; self.view = nil;
    if (self.pdfContext) {
        CGPDFContextClose(self.pdfContext);
        CGContextRelease(self.pdfContext);
        self.pdfContext = nullptr;
    }
    PdfCompletion callback = self.completion;
    uint64_t token = self.token;
    self.completion = nullptr; self.authority = nullptr;
    [activePdfRequests() removeObjectForKey:@(token)];
    if (callback) callback(token, static_cast<uint8_t>(status));
}
- (void)webView:(WKWebView *)view decidePolicyForNavigationAction:(WKNavigationAction *)action decisionHandler:(void (^)(WKNavigationActionPolicy))decision {
    NSURL *url = action.request.URL;
    BOOL allowed = !self.finished && !self.rendering && action.targetFrame.isMainFrame
        && ([url.absoluteString isEqualToString:self.bootstrap.absoluteString] || [url.absoluteString isEqualToString:self.article.absoluteString]);
    decision(allowed ? WKNavigationActionPolicyAllow : WKNavigationActionPolicyCancel);
}
- (void)webView:(WKWebView *)view didFailProvisionalNavigation:(WKNavigation *)navigation withError:(NSError *)error { [self finish:PdfRenderStatus::NavigationFailed]; }
- (void)webView:(WKWebView *)view didFailNavigation:(WKNavigation *)navigation withError:(NSError *)error { [self finish:PdfRenderStatus::NavigationFailed]; }
- (void)webViewWebContentProcessDidTerminate:(WKWebView *)view { [self finish:PdfRenderStatus::RendererTerminated]; }
- (void)webView:(WKWebView *)view didFinishNavigation:(WKNavigation *)navigation {
    if (self.finished || self.rendering || ![view.URL.absoluteString isEqualToString:self.article.absoluteString]) return;
    if (!self.authority || !self.authority(self.token)) { [self finish:PdfRenderStatus::AuthorityExpired]; return; }
    self.rendering = YES;
    if (@available(iOS 14.0, macOS 11.0, *)) {
        // WKPDFConfiguration captures the frozen screen viewport, not WebKit's
        // print-media reload/reflow. Capture bounded page rectangles, then fit
        // them onto A4 without clipping a wide live document or rasterizing text.
        self.sourcePageHeight = self.sourceWidth * (KiwixPaperHeight - 72) / (KiwixPaperWidth - 72);
        self.pageCount = (NSUInteger)std::ceil(self.sourceHeight / self.sourcePageHeight);
        if (!self.pageCount || self.pageCount > 512) { [self finish:PdfRenderStatus::PageLimit]; return; }
        CGDataConsumerRef consumer = CGDataConsumerCreateWithURL((__bridge CFURLRef)self.output);
        CGRect paper = CGRectMake(0, 0, KiwixPaperWidth, KiwixPaperHeight);
        self.pdfContext = consumer ? CGPDFContextCreate(consumer, &paper, nullptr) : nullptr;
        if (consumer) CGDataConsumerRelease(consumer);
        if (!self.pdfContext) { [self finish:PdfRenderStatus::OutputUnavailable]; return; }
        [self capturePage];
    } else { [self finish:PdfRenderStatus::PlatformUnavailable]; }
}
- (void)capturePage {
    if (self.finished) return;
    if (!self.authority || !self.authority(self.token)) { [self finish:PdfRenderStatus::AuthorityExpired]; return; }
    if (self.nextPage == self.pageCount) { [self finish:PdfRenderStatus::Success]; return; }
    if (@available(iOS 14.0, macOS 11.0, *)) {
        double top = self.nextPage * self.sourcePageHeight;
        double height = std::min(self.sourcePageHeight, self.sourceHeight - top);
        WKPDFConfiguration *configuration = [WKPDFConfiguration new];
        configuration.rect = CGRectMake(0, top, self.sourceWidth, height);
        __weak KiwixPdfRequest *weakRequest = self;
        [self.view createPDFWithConfiguration:configuration completionHandler:^(NSData *data, NSError *error) {
            KiwixPdfRequest *request = weakRequest;
            if (!request || request.finished) return;
            if (!request.authority || !request.authority(request.token)) { [request finish:PdfRenderStatus::AuthorityExpired]; return; }
            if (error || !data.length || data.length > KiwixPdfByteLimit) { [request finish:PdfRenderStatus::PageCaptureFailed]; return; }
            CGDataProviderRef provider = CGDataProviderCreateWithCFData((__bridge CFDataRef)data);
            CGPDFDocumentRef document = provider ? CGPDFDocumentCreateWithProvider(provider) : nullptr;
            if (provider) CGDataProviderRelease(provider);
            CGPDFPageRef page = document && CGPDFDocumentGetNumberOfPages(document) == 1 ? CGPDFDocumentGetPage(document, 1) : nullptr;
            if (!page) {
                if (document) CGPDFDocumentRelease(document);
                [request finish:PdfRenderStatus::InvalidPage]; return;
            }
            CGRect source = CGPDFPageGetBoxRect(page, kCGPDFMediaBox);
            if (!std::isfinite(source.size.width) || !std::isfinite(source.size.height) || source.size.width <= 0 || source.size.height <= 0) {
                CGPDFDocumentRelease(document);
                [request finish:PdfRenderStatus::InvalidPageDimensions]; return;
            }
            double scale = (KiwixPaperWidth - 72) / request.sourceWidth;
            CGRect target = CGRectMake(36, KiwixPaperHeight - 36 - height * scale, KiwixPaperWidth - 72, height * scale);
            CGPDFContextBeginPage(request.pdfContext, nullptr);
            CGContextSetRGBFillColor(request.pdfContext, 1, 1, 1, 1);
            CGContextFillRect(request.pdfContext, CGRectMake(0, 0, KiwixPaperWidth, KiwixPaperHeight));
            CGContextSaveGState(request.pdfContext);
            CGContextClipToRect(request.pdfContext, target);
            CGContextConcatCTM(request.pdfContext, CGPDFPageGetDrawingTransform(page, kCGPDFMediaBox, target, 0, true));
            CGContextDrawPDFPage(request.pdfContext, page);
            CGContextRestoreGState(request.pdfContext);
            CGPDFContextEndPage(request.pdfContext);
            CGPDFDocumentRelease(document);
            NSDictionary *attributes = [NSFileManager.defaultManager attributesOfItemAtPath:request.output.path error:nil];
            if ([attributes fileSize] > KiwixPdfByteLimit) { [request finish:PdfRenderStatus::OutputLimit]; return; }
            request.nextPage += 1;
            // Yield between rectangles so privacy/window retirement can revoke
            // authority; no native callback or scratch file outlives its owner.
            dispatch_async(dispatch_get_main_queue(), ^{ [weakRequest capturePage]; });
        }];
    }
}
@end
extern "C" void kiwix_pdf_start(uint64_t token, const char *bootstrap, const char *article, const char *output, uint32_t width, uint32_t height, PdfAuthority authority, PdfCompletion complete) {
    NSString *bootstrapString = [NSString stringWithUTF8String:bootstrap];
    NSString *articleString = [NSString stringWithUTF8String:article];
    NSString *outputString = [NSString stringWithUTF8String:output];
    dispatch_async(dispatch_get_main_queue(), ^{
        KiwixPdfRequest *request = [KiwixPdfRequest new];
        request.token = token; request.authority = authority; request.completion = complete;
        request.bootstrap = [NSURL URLWithString:bootstrapString]; request.article = [NSURL URLWithString:articleString];
        request.output = [NSURL fileURLWithPath:outputString];
        request.sourceWidth = width; request.sourceHeight = height;
        activePdfRequests()[@(token)] = request;
        if (!authority || !authority(token)) { [request finish:PdfRenderStatus::AuthorityExpired]; return; }
        if (!width || width > 16384 || !height || height > 10000000) { [request finish:PdfRenderStatus::InvalidCaptureDimensions]; return; }
        WKWebViewConfiguration *configuration = [WKWebViewConfiguration new];
        configuration.websiteDataStore = WKWebsiteDataStore.nonPersistentDataStore;
        configuration.defaultWebpagePreferences.allowsContentJavaScript = NO;
#if TARGET_OS_IPHONE
        request.view = [[WKWebView alloc] initWithFrame:CGRectMake(0, 0, width, 1) configuration:configuration];
#else
        request.view = [[WKWebView alloc] initWithFrame:NSMakeRect(0, 0, width, 1) configuration:configuration];
#endif
        request.view.navigationDelegate = request;
        __weak KiwixPdfRequest *weakRequest = request;
        request.timer = [NSTimer timerWithTimeInterval:90 repeats:NO block:^(NSTimer *) { [weakRequest finish:PdfRenderStatus::TimedOut]; }];
        [[NSRunLoop mainRunLoop] addTimer:request.timer forMode:NSRunLoopCommonModes];
        [request.view loadRequest:[NSURLRequest requestWithURL:request.bootstrap cachePolicy:NSURLRequestReloadIgnoringLocalCacheData timeoutInterval:60]];
    });
}
extern "C" void kiwix_pdf_cancel(uint64_t token) {
    dispatch_async(dispatch_get_main_queue(), ^{ [activePdfRequests()[@(token)] finish:PdfRenderStatus::Cancelled]; });
}
#if TARGET_OS_IPHONE
// Scratch remains excluded from backup independently of archive-backup opt-in.
extern "C" bool kiwix_pdf_exclude_scratch(const char *path) {
    @autoreleasepool {
        NSString *name = path ? [NSString stringWithUTF8String:path] : nil;
        if (!name) return false;
        NSURL *url = [NSURL fileURLWithPath:name isDirectory:YES].URLByResolvingSymlinksInPath;
        NSMutableArray<NSURL *> *roots = [[[NSFileManager defaultManager] URLsForDirectory:NSCachesDirectory inDomains:NSUserDomainMask] mutableCopy];
        [roots addObject:NSFileManager.defaultManager.temporaryDirectory];
        BOOL owned = NO;
        for (NSURL *root in roots) {
            NSString *prefix = [root.URLByResolvingSymlinksInPath.path stringByAppendingString:@"/"];
            if ([url.path hasPrefix:prefix]) { owned = YES; break; }
        }
        if (!owned) return false;
        NSError *error = nil;
        NSDictionary *values = [url resourceValuesForKeys:@[NSURLIsDirectoryKey] error:&error];
        if (error || ![values[NSURLIsDirectoryKey] boolValue]) return false;
        if (![url setResourceValue:@YES forKey:NSURLIsExcludedFromBackupKey error:&error]) return false;
        values = [url resourceValuesForKeys:@[NSURLIsExcludedFromBackupKey] error:&error];
        return !error && [values[NSURLIsExcludedFromBackupKey] boolValue];
    }
}
#endif
