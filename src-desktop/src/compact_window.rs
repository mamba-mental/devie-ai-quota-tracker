use std::sync::Mutex;
use tauri::{
    AppHandle, LogicalSize, Manager, PhysicalPosition, PhysicalSize, State, WebviewWindow,
};

#[derive(Clone, Copy)]
struct NormalWindow {
    size: PhysicalSize<u32>,
    position: PhysicalPosition<i32>,
    maximized: bool,
}

#[derive(Default)]
pub struct CompactWindow(Mutex<Option<NormalWindow>>);

fn main_window(app: &AppHandle, caller: &WebviewWindow) -> Result<WebviewWindow, String> {
    if !cfg!(target_os = "windows") || caller.label() != "main" {
        return Err("Compact mode is available only in the Windows main window.".into());
    }
    app.get_webview_window("main")
        .ok_or_else(|| "The main window does not exist.".into())
}

fn restore(window: &WebviewWindow, normal: NormalWindow) -> tauri::Result<()> {
    window.set_always_on_top(false)?;
    window.unmaximize()?;
    window.set_min_size(Some(LogicalSize::new(780.0, 560.0)))?;
    window.set_size(normal.size)?;
    window.set_position(normal.position)?;
    if normal.maximized {
        window.maximize()?;
    }
    Ok(())
}

fn clamp_axis(position: i32, origin: i32, available: u32, extent: u32) -> i32 {
    let upper = i64::from(origin) + i64::from(available.saturating_sub(extent));
    i64::from(position)
        .clamp(i64::from(origin), upper)
        .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

fn enter(window: &WebviewWindow) -> tauri::Result<()> {
    window.set_always_on_top(false)?;
    window.set_min_size(Some(LogicalSize::new(320.0, 240.0)))?;
    let monitor = window.current_monitor()?.or(window.primary_monitor()?);
    let mut target = LogicalSize::new(380.0_f64, 480.0_f64);
    if let Some(monitor) = &monitor {
        let scale = window.scale_factor()?;
        let outer = window.outer_size()?;
        let inner = window.inner_size()?;
        let area = monitor.work_area();
        target.width = target
            .width
            .min(
                f64::from(
                    area.size
                        .width
                        .saturating_sub(outer.width.saturating_sub(inner.width)),
                ) / scale,
            )
            .max(320.0);
        target.height = target
            .height
            .min(
                f64::from(
                    area.size
                        .height
                        .saturating_sub(outer.height.saturating_sub(inner.height)),
                ) / scale,
            )
            .max(240.0);
    }
    window.set_size(target)?;
    if let Some(monitor) = monitor {
        let area = monitor.work_area();
        let position = window.outer_position()?;
        let size = window.outer_size()?;
        window.set_position(PhysicalPosition::new(
            clamp_axis(position.x, area.position.x, area.size.width, size.width),
            clamp_axis(position.y, area.position.y, area.size.height, size.height),
        ))?;
    }
    Ok(())
}

#[tauri::command]
pub async fn set_compact_mode(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, CompactWindow>,
    enabled: bool,
) -> Result<(), String> {
    let window = main_window(&app, &window)?;
    let mut saved = state.0.lock().map_err(|error| error.to_string())?;
    if enabled {
        if saved.is_some() {
            return Ok(());
        }
        let maximized = window.is_maximized().map_err(|error| error.to_string())?;
        // Capture the restore geometry, rather than saving the maximized desktop size.
        window.unmaximize().map_err(|error| error.to_string())?;
        let snapshot = (|| -> tauri::Result<NormalWindow> {
            Ok(NormalWindow {
                size: window.inner_size()?,
                position: window.outer_position()?,
                maximized,
            })
        })();
        let normal = snapshot.map_err(|error| {
            if maximized {
                let _ = window.maximize();
            }
            error.to_string()
        })?;
        *saved = Some(normal);
        if let Err(error) = enter(&window) {
            return match restore(&window, normal) {
                Ok(()) => {
                    *saved = None;
                    Err(error.to_string())
                }
                Err(restore_error) => Err(format!(
                    "{error}; restoring the window also failed: {restore_error}"
                )),
            };
        }
    } else if let Some(normal) = *saved {
        restore(&window, normal).map_err(|error| error.to_string())?;
        *saved = None;
    }
    Ok(())
}

#[tauri::command]
pub async fn set_compact_always_on_top(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, CompactWindow>,
    enabled: bool,
) -> Result<(), String> {
    let window = main_window(&app, &window)?;
    let saved = state.0.lock().map_err(|error| error.to_string())?;
    if saved.is_none() {
        return Err("Enter compact mode before changing its pin setting.".into());
    }
    window
        .set_always_on_top(enabled)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::clamp_axis;

    #[test]
    fn compact_window_stays_within_monitor_work_area() {
        assert_eq!(clamp_axis(1800, 0, 1920, 400), 1520);
        assert_eq!(clamp_axis(-2000, -1920, 1920, 400), -1920);
        assert_eq!(clamp_axis(-100, -1920, 1920, 400), -400);
        assert_eq!(clamp_axis(300, 0, 1920, 400), 300);
        assert_eq!(clamp_axis(100, 0, 300, 400), 0);
    }
}
