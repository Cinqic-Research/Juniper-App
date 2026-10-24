//! Desktop main-window placement.
//!
//! The configured window (1000x700 on Windows, 1280x820 elsewhere) is larger
//! than the usable area of a 1366x768 laptop display once the taskbar is
//! subtracted, and the window system places it under the taskbar rather than
//! shrinking it; the composer at the bottom of Chat then sits behind the
//! taskbar. The `center` option in the window configuration also had no
//! visible effect on Windows 11. So after the window exists, it is fitted to
//! the work area of the monitor it opened on and centred there. The
//! configured logical minimum is preserved where it fits; on smaller or
//! scaled work areas it is lowered just enough to keep the frame accessible.

use tauri::{LogicalSize, Manager, PhysicalPosition, PhysicalSize, Runtime};

fn client_capacity(work: (u32, u32), outer: (u32, u32), inner: (u32, u32)) -> (u32, u32) {
    (
        work.0
            .saturating_sub(outer.0.saturating_sub(inner.0))
            .max(1),
        work.1
            .saturating_sub(outer.1.saturating_sub(inner.1))
            .max(1),
    )
}

fn fitted_minimum(
    work: (u32, u32),
    outer: (u32, u32),
    inner: (u32, u32),
    configured: (u32, u32),
) -> (u32, u32) {
    let available = client_capacity(work, outer, inner);
    (configured.0.min(available.0), configured.1.min(available.1))
}

fn centered_position(
    work_position: (i32, i32),
    work_size: (u32, u32),
    outer: (u32, u32),
) -> (i32, i32) {
    (
        work_position.0 + (work_size.0.saturating_sub(outer.0) / 2) as i32,
        work_position.1 + (work_size.1.saturating_sub(outer.1) / 2) as i32,
    )
}

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
    let available = client_capacity(work_size, outer, inner);
    let fitted = (
        inner.0.min(available.0).max(1),
        inner.1.min(available.1).max(1),
    );
    let total = (fitted.0 + frame.0, fitted.1 + frame.1);
    let position = centered_position(work_position, work_size, total);
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
    let (Ok(scale), Some(config)) = (
        window.scale_factor(),
        app.config()
            .app
            .windows
            .iter()
            .find(|config| config.label == "main"),
    ) else {
        return;
    };
    let configured: PhysicalSize<u32> = LogicalSize::new(
        config.min_width.unwrap_or(0.0),
        config.min_height.unwrap_or(0.0),
    )
    .to_physical(scale);
    let minimum = fitted_minimum(
        (work.size.width, work.size.height),
        (outer.width, outer.height),
        (inner.width, inner.height),
        (configured.width, configured.height),
    );
    if minimum != (configured.width, configured.height)
        && window
            .set_min_size(Some(LogicalSize::new(
                f64::from(minimum.0) / scale,
                f64::from(minimum.1) / scale,
            )))
            .is_err()
    {
        return;
    }
    let (size, _) = fitted_placement(
        (work.position.x, work.position.y),
        (work.size.width, work.size.height),
        (outer.width, outer.height),
        (inner.width, inner.height),
    );
    if size != (inner.width, inner.height)
        && window.set_size(PhysicalSize::new(size.0, size.1)).is_err()
    {
        return;
    }
    // The window manager can enforce constraints or adjust decorations. Centre
    // the actual frame rather than assuming it accepted the requested size.
    let Ok(actual) = window.outer_size() else {
        return;
    };
    let position = centered_position(
        (work.position.x, work.position.y),
        (work.size.width, work.size.height),
        (actual.width, actual.height),
    );
    let _ = window.set_position(PhysicalPosition::new(position.0, position.1));
}

#[cfg(test)]
mod tests {
    use super::{centered_position, fitted_minimum, fitted_placement};
    use tauri::LogicalSize;

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

    #[test]
    fn scaled_minimum_does_not_force_the_window_under_the_taskbar() {
        // At 125%, the configured 600 logical pixel minimum is 750 physical
        // pixels before decorations; the CINQTOP work area is only 720 high.
        let inner = (1250, 875);
        let outer = (1270, 924);
        let minimum = fitted_minimum((1366, 720), outer, inner, (450, 750));
        assert_eq!(minimum, (450, 671));
        let (size, _) = fitted_placement((0, 0), (1366, 720), outer, inner);
        assert_eq!(size, (1250, 671));
        assert!(size.0 >= minimum.0 && size.1 >= minimum.1);
    }

    #[test]
    fn geometry_fits_taskbar_offsets_scales_and_small_work_areas() {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let client = LogicalSize::new(1000.0, 700.0).to_physical::<u32>(scale);
            let frame = LogicalSize::new(16.0, 39.0).to_physical::<u32>(scale);
            let configured = LogicalSize::new(360.0, 600.0).to_physical::<u32>(scale);
            let inner = (client.width, client.height);
            let outer = (client.width + frame.width, client.height + frame.height);
            for (origin, work) in [
                ((0, 0), (1366, 720)),        // Bottom taskbar.
                ((0, 48), (1366, 720)),       // Top taskbar.
                ((48, 0), (1318, 768)),       // Left taskbar.
                ((0, 0), (1318, 768)),        // Right taskbar.
                ((-1366, -768), (1366, 720)), // Negative monitor coordinates.
                ((1920, 0), (1366, 720)),     // Positive monitor coordinates.
                ((-300, 48), (300, 300)),     // Smaller than both configured minima.
            ] {
                let minimum =
                    fitted_minimum(work, outer, inner, (configured.width, configured.height));
                let (size, position) = fitted_placement(origin, work, outer, inner);
                // Simulate the window system applying its minimum: the fitted
                // client must already satisfy it, so no hidden enlargement occurs.
                assert!(size.0 >= minimum.0 && size.1 >= minimum.1);
                let total = (size.0 + frame.width, size.1 + frame.height);
                assert!(total.0 <= work.0 && total.1 <= work.1);
                assert!(position.0 >= origin.0 && position.1 >= origin.1);
                assert!(
                    i64::from(position.0) + i64::from(total.0)
                        <= i64::from(origin.0) + i64::from(work.0)
                );
                assert!(
                    i64::from(position.1) + i64::from(total.1)
                        <= i64::from(origin.1) + i64::from(work.1)
                );
                assert_eq!(minimum.0, configured.width.min(work.0 - frame.width));
                assert_eq!(minimum.1, configured.height.min(work.1 - frame.height));
            }
        }
    }

    #[test]
    fn an_existing_smaller_window_is_not_enlarged_and_the_minimum_is_preserved() {
        let inner = (700, 620);
        let outer = (716, 659);
        let work = (1920, 1040);
        assert_eq!(fitted_minimum(work, outer, inner, (360, 600)), (360, 600));
        let (size, position) = fitted_placement((0, 0), work, outer, inner);
        assert_eq!(size, inner);
        assert_eq!(position, (602, 190));
    }

    #[test]
    fn centering_uses_the_frame_actually_accepted_by_the_window_system() {
        assert_eq!(
            centered_position((-1920, 48), (1920, 1040), (1200, 800)),
            (-1560, 168)
        );
    }
}
