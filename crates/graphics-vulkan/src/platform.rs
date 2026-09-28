use std::sync::{Mutex, OnceLock};

#[derive(Clone, Copy)]
pub enum Surface {
    Xlib {
        display: usize,
        window: u64,
        width: u32,
        height: u32,
    },
    Wayland {
        display: usize,
        surface: usize,
        width: u32,
        height: u32,
    },
}

static SURFACE: OnceLock<Mutex<Option<Surface>>> = OnceLock::new();

fn state() -> &'static Mutex<Option<Surface>> {
    SURFACE.get_or_init(|| Mutex::new(None))
}

pub fn set_surface(surface: Option<Surface>) {
    *state().lock().unwrap_or_else(|error| error.into_inner()) = surface;
}

pub fn resize_surface(width: u32, height: u32) {
    let mut surface = state().lock().unwrap_or_else(|error| error.into_inner());
    match surface.as_mut() {
        Some(Surface::Xlib {
            width: w,
            height: h,
            ..
        })
        | Some(Surface::Wayland {
            width: w,
            height: h,
            ..
        }) => {
            *w = width;
            *h = height;
        }
        None => {}
    }
}

pub(crate) fn current_surface() -> Option<Surface> {
    *state().lock().unwrap_or_else(|error| error.into_inner())
}
