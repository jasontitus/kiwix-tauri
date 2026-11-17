// SPDX-License-Identifier: GPL-3.0-or-later
#import <Foundation/Foundation.h>
#import "PdfExportBridge.h"
#import <os/lock.h>
static os_unfair_lock pdfCommitLock = OS_UNFAIR_LOCK_INIT;
static KiwixPdfCommit pdfCommit;
void kiwix_pdf_export_install(KiwixPdfCommit commit) {
    os_unfair_lock_lock(&pdfCommitLock);
    pdfCommit = commit;
    os_unfair_lock_unlock(&pdfCommitLock);
}
static void invoke_pdf_action(void *context) {
    void (^action)(void) = (__bridge void (^)(void))context;
    action();
}
bool kiwix_pdf_export_commit(const char *token, void (^action)(void)) {
    // The borrowed block stays on this synchronous stack. Never copy, retain,
    // dispatch, or store it in native/Rust registries.
    os_unfair_lock_lock(&pdfCommitLock);
    KiwixPdfCommit commit = pdfCommit;
    os_unfair_lock_unlock(&pdfCommitLock);
    return token && action && commit && commit(token, (__bridge void *)action, invoke_pdf_action);
}
