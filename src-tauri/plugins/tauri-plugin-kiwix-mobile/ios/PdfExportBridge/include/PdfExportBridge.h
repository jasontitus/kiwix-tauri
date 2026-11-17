// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once
#include <stdbool.h>
typedef bool (*KiwixPdfCommit)(const char * _Nonnull token, void * _Nonnull context, void (* _Nonnull action)(void * _Nonnull));
/// Trusted Rust installs its callback before admitting any PDF export. Keeping
/// this module self-contained also preserves standalone Swift policy tests.
void kiwix_pdf_export_install(KiwixPdfCommit _Nonnull commit);
/// Synchronous and nonescaping. Unknown/consumed authority never runs action.
bool kiwix_pdf_export_commit(const char * _Nonnull token, void (^ _Nonnull action)(void));
