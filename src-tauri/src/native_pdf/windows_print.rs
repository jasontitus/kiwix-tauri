// SPDX-License-Identifier: GPL-3.0-or-later
//! Public Win32 printer selection followed by WebView2's public Print API.
//! No policy lock is held while the user considers the modal dialog.
use std::{cell::RefCell, collections::HashMap, sync::atomic::{AtomicBool, AtomicIsize, Ordering}};
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use windows::{core::{HSTRING, Interface}, Win32::{
    Foundation::{GlobalFree, HWND, LPARAM, WPARAM},
    Graphics::Gdi::{DeleteDC, GetDeviceCaps, DEVMODEW, DM_COLOR, DM_DUPLEX, DM_ORIENTATION, DM_SCALE, DMORIENT_LANDSCAPE, LOGPIXELSX, LOGPIXELSY, PHYSICALWIDTH, PHYSICALHEIGHT},
    System::Memory::{GlobalLock, GlobalSize, GlobalUnlock},
    UI::{Controls::Dialogs::*, WindowsAndMessaging::{PostMessageW, WM_COMMAND, WM_INITDIALOG, WM_NCDESTROY}},
}};
#[derive(Default)]
pub struct Dialog { hwnd: AtomicIsize, cancelled: AtomicBool }
impl Dialog {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        let hwnd = self.hwnd.load(Ordering::SeqCst);
        // SAFETY: `hwnd` was published by `hook` and cleared on WM_NCDESTROY,
        // but a concurrent teardown can recycle the value. The post is
        // best-effort and carries no borrowed Rust pointer, so a stale handle
        // cannot reach our data.
        if hwnd != 0 { unsafe { let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_COMMAND, WPARAM(2), LPARAM(0)); } }
    }
}
thread_local! { static HOOKS: RefCell<HashMap<isize, *const Dialog>> = RefCell::new(HashMap::new()); }
unsafe extern "system" fn hook(hwnd: HWND, message: u32, _wparam: WPARAM, lparam: LPARAM) -> usize {
    if message == WM_INITDIALOG {
        // SAFETY: during WM_INITDIALOG `lParam` is the live PRINTDLGW that
        // `choose` passed to PrintDlgW; only `lCustData` is read here, before
        // the call writes its selection back into the structure.
        let print = unsafe { &*(lparam.0 as *const PRINTDLGW) };
        let dialog = print.lCustData.0 as *const Dialog;
        if !dialog.is_null() {
            HOOKS.with(|hooks| hooks.borrow_mut().insert(hwnd.0 as isize, dialog));
            // SAFETY: `lCustData` holds the `&Dialog` owned by `choose`, which
            // keeps it alive until PrintDlgW returns; the pointer was checked
            // non-null and is used only while that reference is live.
            unsafe { &*dialog }.hwnd.store(hwnd.0 as isize, Ordering::SeqCst);
            // SAFETY: same live `&Dialog`; `cancel` only posts to the HWND just
            // stored above and never outlives `choose`.
            if unsafe { &*dialog }.cancelled.load(Ordering::SeqCst) { unsafe { &*dialog }.cancel(); }
        }
    } else if message == WM_NCDESTROY {
        if let Some(dialog) = HOOKS.with(|hooks| hooks.borrow_mut().remove(&(hwnd.0 as isize))) {
            // SAFETY: entries are inserted with a non-null `&Dialog` and removed
            // here before `choose` returns, so the pointee is still alive.
            unsafe { &*dialog }.hwnd.store(0, Ordering::SeqCst);
        }
    }
    0
}
struct Selection(PRINTDLGW);
impl Drop for Selection {
    // SAFETY: the handles were allocated by PrintDlgW for this Selection and
    // are freed exactly once here after the dialog has returned; invalid
    // handles are skipped and nothing reads them afterwards.
    fn drop(&mut self) { unsafe {
        let mode = self.0.hDevMode; let names = self.0.hDevNames; let dc = self.0.hDC;
        if !mode.is_invalid() { let _ = GlobalFree(Some(mode)); }
        if !names.is_invalid() { let _ = GlobalFree(Some(names)); }
        if !dc.is_invalid() { let _ = DeleteDC(dc); }
    } }
}
pub fn choose(core: &ICoreWebView2, owner: HWND, dialog: &Dialog, width: u32) -> Result<Option<ICoreWebView2PrintSettings>, String> {
    if dialog.cancelled.load(Ordering::SeqCst) { return Err("Article printing was cancelled".into()); }
    let mut selection = Selection(PRINTDLGW {
        lStructSize: std::mem::size_of::<PRINTDLGW>() as u32,
        hwndOwner: owner,
        Flags: PD_NOSELECTION | PD_RETURNIC | PD_ENABLEPRINTHOOK | PD_HIDEPRINTTOFILE,
        nMinPage: 1, nMaxPage: 512, nFromPage: 1, nToPage: 512, nCopies: 1,
        lCustData: LPARAM(dialog as *const Dialog as isize), lpfnPrintHook: Some(hook),
        ..Default::default()
    });
    // SAFETY: `selection.0` is a fully initialized PRINTDLGW (size and owner
    // window set) on this stack frame and outlives the modal call; PrintDlgW
    // invokes `hook` on this same thread.
    let selected = unsafe { PrintDlgW(&mut selection.0) }.as_bool();
    dialog.hwnd.store(0, Ordering::SeqCst);
    if dialog.cancelled.load(Ordering::SeqCst) { return Err("Article printing was cancelled".into()); }
    if !selected {
        // SAFETY: reads the thread-local dialog error state left by the
        // PrintDlgW call that just returned on this thread; no pointer is
        // involved.
        let error = unsafe { CommDlgExtendedError() }.0;
        return if error == 0 { Ok(None) } else { Err(format!("The Windows print dialog failed ({error})")) };
    }
    // SAFETY: `settings` reads the still-owned PRINTDLG handles, which
    // `selection` frees only when it drops after this call returns.
    unsafe { settings(core, &selection.0, width).map(Some).map_err(|error| format!("Cannot apply the selected printer settings: {error}")) }
}
// SAFETY: every handle in `print` came from PrintDlgW and is owned by the live
// `selection`; each global block is sized by GlobalSize, locked and unlocked
// once here, and its unaligned contents are read only inside those bounds.
unsafe fn settings(core: &ICoreWebView2, print: &PRINTDLGW, width: u32) -> Result<ICoreWebView2PrintSettings, String> {
    unsafe {
        let flags = print.Flags;
        // DEVNAMES offsets are UTF-16 units into the bounded global allocation,
        // not pointers supplied by archive markup.
        let size = GlobalSize(print.hDevNames) / 2;
        if size < std::mem::size_of::<DEVNAMES>() / 2 { return Err("Missing printer identity".into()); }
        let names = GlobalLock(print.hDevNames);
        if names.is_null() { return Err("Missing printer identity".into()); }
        let name_result = (|| {
            let header = std::ptr::read_unaligned(names.cast::<DEVNAMES>());
            let offset = header.wDeviceOffset as usize;
            if offset >= size { return Err("Invalid printer identity".to_string()); }
            let text = std::slice::from_raw_parts(names.cast::<u16>().add(offset), size - offset);
            let end = text.iter().position(|ch| *ch == 0).ok_or("Invalid printer identity")?;
            if end == 0 { return Err("No printer was selected".into()); }
            String::from_utf16(&text[..end]).map_err(|_| "Invalid printer name".into())
        })();
        let _ = GlobalUnlock(print.hDevNames);
        let name = name_result?;
        if GlobalSize(print.hDevMode) < std::mem::size_of::<DEVMODEW>() { return Err("Missing printer configuration".into()); }
        let mode = GlobalLock(print.hDevMode);
        if mode.is_null() { return Err("Missing printer configuration".into()); }
        let mode = std::ptr::read_unaligned(mode.cast::<DEVMODEW>());
        let _ = GlobalUnlock(print.hDevMode);
        let environment = core.cast::<ICoreWebView2_2>().map_err(|error| error.to_string())?.Environment().map_err(|error| error.to_string())?;
        let settings = environment.cast::<ICoreWebView2Environment6>().map_err(|error| error.to_string())?.CreatePrintSettings().map_err(|error| error.to_string())?;
        let extended = settings.cast::<ICoreWebView2PrintSettings2>().map_err(|error| error.to_string())?;
        extended.SetPrinterName(&HSTRING::from(name)).map_err(|error| error.to_string())?;
        extended.SetCopies(i32::from(print.nCopies.max(1))).map_err(|error| error.to_string())?;
        extended.SetCollation(if flags.contains(PD_COLLATE) { COREWEBVIEW2_PRINT_COLLATION_COLLATED } else { COREWEBVIEW2_PRINT_COLLATION_UNCOLLATED }).map_err(|error| error.to_string())?;
        if flags.contains(PD_PAGENUMS) {
            let first = print.nFromPage; let last = print.nToPage;
            if first == 0 || last < first { return Err("Invalid print page range".into()); }
            extended.SetPageRanges(&HSTRING::from(format!("{first}-{last}"))).map_err(|error| error.to_string())?;
        }
        if mode.dmFields.contains(DM_COLOR) {
            extended.SetColorMode(if mode.dmColor.0 == 1 { COREWEBVIEW2_PRINT_COLOR_MODE_GRAYSCALE } else { COREWEBVIEW2_PRINT_COLOR_MODE_COLOR }).map_err(|error| error.to_string())?;
        }
        if mode.dmFields.contains(DM_DUPLEX) {
            let duplex = match mode.dmDuplex.0 { 1 => COREWEBVIEW2_PRINT_DUPLEX_ONE_SIDED, 2 => COREWEBVIEW2_PRINT_DUPLEX_TWO_SIDED_LONG_EDGE, 3 => COREWEBVIEW2_PRINT_DUPLEX_TWO_SIDED_SHORT_EDGE, _ => return Err("Unsupported printer duplex setting".into()) };
            extended.SetDuplex(duplex).map_err(|error| error.to_string())?;
        }
        let dpi_x = GetDeviceCaps(Some(print.hDC), LOGPIXELSX);
        let dpi_y = GetDeviceCaps(Some(print.hDC), LOGPIXELSY);
        if dpi_x <= 0 || dpi_y <= 0 { return Err("Invalid printer resolution".into()); }
        let physical_width = f64::from(GetDeviceCaps(Some(print.hDC), PHYSICALWIDTH)) / f64::from(dpi_x);
        let physical_height = f64::from(GetDeviceCaps(Some(print.hDC), PHYSICALHEIGHT)) / f64::from(dpi_y);
        if physical_width <= 0.0 || physical_height <= 0.0 { return Err("Invalid printer paper dimensions".into()); }
        let landscape = mode.dmFields.contains(DM_ORIENTATION) && mode.Anonymous1.Anonymous1.dmOrientation as u32 == DMORIENT_LANDSCAPE;
        settings.SetOrientation(if landscape { COREWEBVIEW2_PRINT_ORIENTATION_LANDSCAPE } else { COREWEBVIEW2_PRINT_ORIENTATION_PORTRAIT }).map_err(|error| error.to_string())?;
        extended.SetMediaSize(COREWEBVIEW2_PRINT_MEDIA_SIZE_CUSTOM).map_err(|error| error.to_string())?;
        settings.SetPageWidth(if landscape { physical_height } else { physical_width }).map_err(|error| error.to_string())?;
        settings.SetPageHeight(if landscape { physical_width } else { physical_height }).map_err(|error| error.to_string())?;
        settings.SetMarginTop(0.5).map_err(|error| error.to_string())?;
        settings.SetMarginBottom(0.5).map_err(|error| error.to_string())?;
        settings.SetMarginLeft(0.5).map_err(|error| error.to_string())?;
        settings.SetMarginRight(0.5).map_err(|error| error.to_string())?;
        let fit = ((physical_width - 1.0) * 96.0 / f64::from(width)).min(1.0);
        let scale = if mode.dmFields.contains(DM_SCALE) { f64::from(mode.Anonymous1.Anonymous1.dmScale) / 100.0 } else { 1.0 };
        if !(0.1..=2.0).contains(&(scale * fit)) { return Err("The live article is too wide for the selected printer scale".into()); }
        settings.SetScaleFactor(scale * fit).map_err(|error| error.to_string())?;
        settings.SetShouldPrintBackgrounds(true).map_err(|error| error.to_string())?;
        Ok(settings)
    }
}

pub fn pdf_settings(core: &ICoreWebView2, width: u32) -> Result<ICoreWebView2PrintSettings, String> {
    let scale = (((595.28 - 72.0) / 72.0 * 96.0) / f64::from(width)).min(1.0);
    if scale < 0.1 { return Err("The live article is too wide for WebView2 PDF capture".into()); }
    // SAFETY: `core` is a live WebView2 interface borrowed from the caller for
    // this call; `CreatePrintSettings` returns a reference-counted interface
    // that the caller then owns.
    (|| -> windows::core::Result<ICoreWebView2PrintSettings> { unsafe {
        let environment = core.cast::<ICoreWebView2_2>()?.Environment()?;
        let settings = environment.cast::<ICoreWebView2Environment6>()?.CreatePrintSettings()?;
        settings.SetPageWidth(595.28 / 72.0)?;
        settings.SetPageHeight(841.89 / 72.0)?;
        settings.SetMarginTop(0.5)?; settings.SetMarginBottom(0.5)?;
        settings.SetMarginLeft(0.5)?; settings.SetMarginRight(0.5)?;
        settings.SetScaleFactor(scale)?;
        settings.SetShouldPrintBackgrounds(true)?;
        Ok(settings)
    } })().map_err(|error| format!("Creating native PDF page settings: {error}"))
}
