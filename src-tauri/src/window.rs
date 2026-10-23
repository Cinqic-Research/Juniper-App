//! Desktop main-window placement.
//!
//! The configured window (1000x700 on Windows, 1280x820 elsewhere) is larger
//! than the usable area of a 1366x768 laptop display once the taskbar is
//! subtracted, and the window system places it under the taskbar rather than
//! shrinking it; the composer at the bottom of Chat then sits behind the
//! taskbar. The `center` option in the window configuration also had no
//! visible effect on Windows 11. So after the window exists, it is fitted to
//! the work area of the monitor it opened on and centred there. The minimum
//! size constraints still apply: a display smaller than them keeps the
//! minimum and is clipped, as before.

use tauri::{Manager, PhysicalPosition, PhysicalSize, Runtime};

/// Size and position (both physical pixels) that keep a window inside a
/// monitor's work area and centre it there. `inner` and `outer` are the
/// window's current client and frame sizes; the frame insets they imply are
/// preserved so the whole frame fits.
pub fn fitted_placement(
    work_position: (i32, i32),
    work_size: (u32, u32),
    outer: (u32, u32),
    inner: (u32, u32),
) -> ((u32, u32), (i32, i32)) {
    let frame = (
        outer.0.saturating_sub(inner.0),
        outer.1.saturating_sub(inner.1),
    );
    let fitted = (
        inner.0.min(work_size.0.saturating_sub(frame.0)).max(1),
        inner.1.min(work_size.1.saturating_sub(frame.1)).max(1),
    );
    let total = (fitted.0 + frame.0, fitted.1 + frame.1);
    let position = (
        work_position.0 + (i64::from(work_size.0) - i64::from(total.0)).max(0) as i32 / 2,
        work_position.1 + (i64::from(work_size.1) - i64::from(total.1)).max(0) as i32 / 2,
    );
    (fitted, position)
}

/// Fits the main window to its monitor's work area. Every failure is ignored:
/// a window that cannot be measured is left where the window system put it.
pub fn fit_main_window<R: Runtime>(app: &tauri::AppHandle<R>) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let Ok(Some(monitor)) = window.current_monitor() else {
        return;
    };
    let (Ok(outer), Ok(inner)) = (window.outer_size(), window.inner_size()) else {
        return;
    };
    let work = monitor.work_area();
    let (size, position) = fitted_placement(
        (work.position.x, work.position.y),
        (work.size.width, work.size.height),
        (outer.width, outer.height),
        (inner.width, inner.height),
    );
    if size != (inner.width, inner.height) {
        let _ = window.set_size(PhysicalSize::new(size.0, size.1));
    }
    let _ = window.set_position(PhysicalPosition::new(position.0, position.1));
}

#[cfg(test)]
mod tests {
    use super::fitted_placement;

    #[test]
    fn a_window_that_fits_is_only_centred() {
        let (size, position) = fitted_placement((0, 0), (1920, 1040), (1016, 739), (1000, 700));
        assert_eq!(size, (1000, 700));
        assert_eq!(position, ((1920 - 1016) / 2, (1040 - 739) / 2));
    }

    #[test]
    fn a_window_taller_than_the_work_area_is_shrunk_to_fit_with_its_frame() {
        // CINQTOP: 1366x768 panel, 48 px taskbar, 1000x700 client in a 1016x739 frame.
        let (size, position) = fitted_placement((0, 0), (1366, 720), (1016, 739), (1000, 700));
        assert_eq!(size, (1000, 681));
        assert_eq!(position, ((1366 - 1016) / 2, 0));
    }

    #[test]
    fn a_secondary_monitor_offset_is_kept() {
        let (size, position) = fitted_placement((1920, 0), (1366, 720), (1016, 739), (1000, 700));
        assert_eq!(size, (1000, 681));
        assert_eq!(position, (1920 + (1366 - 1016) / 2, 0));
    }

    #[test]
    fn unmeasurable_frames_never_underflow() {
        let (size, position) = fitted_placement((0, 0), (300, 200), (10, 10), (1000, 700));
        assert_eq!(size, (300, 200));
        assert_eq!(position, (0, 0));
    }
}
