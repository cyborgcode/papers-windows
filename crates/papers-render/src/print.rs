//! Printing through the Windows print dialog.
//!
//! Each page is rendered on the render thread at up to 300 dpi (in day colours) and sent
//! to the printer with `StretchDIBits`, fitted to the printable area and turned to match
//! the paper orientation.

use pdfium_render::prelude::*;

#[cfg(not(windows))]
pub(crate) fn print(_document: &PdfDocument, _title: &str, _owner: isize) -> Result<bool, String> {
    Err("Printing is only available on Windows.".into())
}

#[cfg(windows)]
pub(crate) fn print(document: &PdfDocument, title: &str, owner: isize) -> Result<bool, String> {
    use std::mem::{size_of, zeroed};
    use windows_sys::Win32::Foundation::{GlobalFree, HWND};
    use windows_sys::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteDC, GetDeviceCaps, HORZRES, LOGPIXELSX, LOGPIXELSY,
        SRCCOPY, SetStretchBltMode, HALFTONE, StretchDIBits, VERTRES,
    };
    use windows_sys::Win32::Storage::Xps::{DOCINFOW, EndDoc, EndPage, StartDocW, StartPage, AbortDoc};
    use windows_sys::Win32::UI::Controls::Dialogs::{
        CommDlgExtendedError, PD_NOSELECTION, PD_PAGENUMS, PD_RETURNDC, PD_USEDEVMODECOPIESANDCOLLATE, PRINTDLGW,
        PrintDlgW,
    };

    let pages = document.pages();
    let count = pages.len() as u32;
    if count == 0 {
        return Err("The document has no pages.".into());
    }
    let mut dialog: PRINTDLGW = unsafe { zeroed() };
    dialog.lStructSize = size_of::<PRINTDLGW>() as u32;
    dialog.hwndOwner = owner as HWND;
    dialog.Flags = PD_RETURNDC | PD_USEDEVMODECOPIESANDCOLLATE | PD_NOSELECTION;
    dialog.nMinPage = 1;
    dialog.nMaxPage = count.min(u16::MAX as u32) as u16;
    dialog.nFromPage = 1;
    dialog.nToPage = dialog.nMaxPage;
    dialog.nCopies = 1;
    let ok = unsafe { PrintDlgW(&mut dialog) };
    let free = |d: &PRINTDLGW| unsafe {
        if !d.hDevMode.is_null() {
            GlobalFree(d.hDevMode);
        }
        if !d.hDevNames.is_null() {
            GlobalFree(d.hDevNames);
        }
    };
    if ok == 0 {
        let code = unsafe { CommDlgExtendedError() };
        free(&dialog);
        return if code == 0 { Ok(false) } else { Err(format!("The print dialog failed (error {code:#x}).")) };
    }
    let hdc = dialog.hDC;
    let (first, last) = if dialog.Flags & PD_PAGENUMS != 0 {
        (dialog.nFromPage.max(1) as u32, (dialog.nToPage as u32).min(count))
    } else {
        (1, count)
    };

    let result = (|| -> Result<(), String> {
        if hdc.is_null() {
            return Err("No printer was selected.".into());
        }
        let name: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
        let mut info: DOCINFOW = unsafe { zeroed() };
        info.cbSize = size_of::<DOCINFOW>() as i32;
        info.lpszDocName = name.as_ptr();
        if unsafe { StartDocW(hdc, &info) } <= 0 {
            return Err("The printer did not accept the document.".into());
        }
        let (area_w, area_h) = unsafe { (GetDeviceCaps(hdc, HORZRES as _), GetDeviceCaps(hdc, VERTRES as _)) };
        let (dpi_x, dpi_y) = unsafe { (GetDeviceCaps(hdc, LOGPIXELSX as _).max(72), GetDeviceCaps(hdc, LOGPIXELSY as _).max(72)) };
        for index in first..=last {
            let page = pages.get((index - 1) as PdfPageIndex).map_err(|e| e.to_string())?;
            let (pw, ph) = (page.width().value, page.height().value);
            // Turn landscape pages on portrait paper (and the other way round).
            let turn = (pw > ph) != (area_w > area_h);
            let (w_in, h_in) = if turn { (ph / 72.0, pw / 72.0) } else { (pw / 72.0, ph / 72.0) };
            let fit = (area_w as f32 / (w_in * dpi_x as f32)).min(area_h as f32 / (h_in * dpi_y as f32));
            let scale = fit.min(1.0);
            let dest_w = (w_in * dpi_x as f32 * scale).round() as i32;
            let dest_h = (h_in * dpi_y as f32 * scale).round() as i32;
            let x = (area_w - dest_w) / 2;
            let y = (area_h - dest_h) / 2;
            // Render at the printer's resolution, but no more than 300 dpi.
            let dpi = (dpi_x.min(dpi_y) as f32).min(300.0);
            let render_w = ((w_in * dpi * scale).round() as i32).clamp(1, dest_w.max(1));
            let render_h = ((h_in * dpi * scale).round() as i32).clamp(1, dest_h.max(1));
            let rotation = if turn { PdfPageRenderRotation::Degrees90 } else { PdfPageRenderRotation::None };
            let config = PdfRenderConfig::new()
                .set_fixed_size(render_w, render_h)
                .rotate(rotation, false)
                .render_annotations(true)
                .render_form_data(true)
                .use_print_quality(true)
                .set_clear_color(PdfColor::WHITE);
            let mut bitmap = PdfBitmap::empty(render_w, render_h, PdfBitmapFormat::BGRA).map_err(|e| e.to_string())?;
            page.render_into_bitmap_with_config(&mut bitmap, &config).map_err(|e| e.to_string())?;
            let bytes = bitmap.as_raw_bytes();
            let stride = bytes.len() / render_h as usize;
            if stride != render_w as usize * 4 {
                return Err("Unexpected bitmap layout.".into());
            }
            let mut header: BITMAPINFO = unsafe { zeroed() };
            header.bmiHeader = BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: render_w,
                biHeight: -render_h, // top-down rows
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..unsafe { zeroed() }
            };
            unsafe {
                if StartPage(hdc) <= 0 {
                    return Err("The printer stopped accepting pages.".into());
                }
                SetStretchBltMode(hdc, HALFTONE);
                StretchDIBits(
                    hdc,
                    x,
                    y,
                    dest_w,
                    dest_h,
                    0,
                    0,
                    render_w,
                    render_h,
                    bytes.as_ptr().cast(),
                    &header,
                    DIB_RGB_COLORS,
                    SRCCOPY,
                );
                if EndPage(hdc) <= 0 {
                    return Err("The printer stopped accepting pages.".into());
                }
            }
        }
        Ok(())
    })();
    unsafe {
        if result.is_ok() {
            EndDoc(hdc);
        } else if !hdc.is_null() {
            AbortDoc(hdc);
        }
        if !hdc.is_null() {
            DeleteDC(hdc);
        }
    }
    free(&dialog);
    result.map(|()| true)
}
