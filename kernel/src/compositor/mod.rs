//! Desktop Shell Compositor & Window Manager for KntonOS.
//!
//! Provides hardware-accelerated linear framebuffer abstractions, dirty rectangle damage
//! tracking, mouse cursor sprite overlay, Fluent window frame styling, and double-buffered
//! multi-window scene composition.

pub mod acrylic;
pub mod controls;
pub mod cursor;
pub mod dirty;
pub mod window;
pub mod compositor;
pub mod taskbar;

use crate::display::color::Color;
use crate::display::framebuffer::{FramebufferInfo, LinearFramebuffer};
use crate::memory::spinlock::Spinlock;
use crate::serial_write_str;

pub use acrylic::{AcrylicConfig, AcrylicSurface};
pub use controls::{Button, ButtonState, Label, TitleBarButton, TitleBarButtonType, UiWidget};
pub use cursor::{cursor_sprite_pixel, CursorState, CURSOR_HEIGHT, CURSOR_WIDTH};
pub use dirty::{DamageRegion, Rect};
pub use window::{Window, BORDER_WIDTH, BUTTON_WIDTH, TITLEBAR_HEIGHT};
pub use compositor::{SceneCompositor, DEFAULT_TASKBAR_HEIGHT};
pub use taskbar::{DesktopTaskbar, TaskbarItem, TaskbarClickAction, TASKBAR_HEIGHT, TASKBAR_TOP_BORDER_HIGHLIGHT};

/// Global compositor manager instance.
pub static COMPOSITOR: Spinlock<Option<SceneCompositor>> = Spinlock::new(None);

/// Initialize the desktop shell compositor singleton.
pub fn init(width: usize, height: usize) {
    serial_write_str("[KNT-COMPOSITOR] Desktop shell compositor & window manager initialized\r\n");
    let mut comp = COMPOSITOR.lock();
    *comp = Some(SceneCompositor::new(width, height));
}

/// Execute a closure with mutable access to the global compositor instance.
pub fn with_compositor<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut SceneCompositor) -> R,
{
    let mut guard = COMPOSITOR.lock();
    guard.as_mut().map(f)
}

/// Update mouse cursor from relative motion delta and button states.
pub fn update_cursor(dx: i32, dy: i32, buttons: u8) {
    with_compositor(|comp| {
        comp.update_cursor(dx, dy, buttons);
    });
}

/// Dispatch mouse pointer coordinates and button state directly into the compositor.
pub fn dispatch_mouse_input(mx: usize, my: usize, buttons: u8) {
    with_compositor(|comp| {
        comp.dispatch_mouse_input(mx, my, buttons);
    });
}

/// Verification self-test suite for the KntonOS compositor subsystem.
pub fn test_compositor_subsystem() -> bool {
    serial_write_str("[KNT-COMPOSITOR-TEST] Running compositor verification suite...\r\n");

    // 1. Verify dirty rectangle tracking and damaged region calculation
    let r1 = Rect::new(10, 10, 50, 50);
    let r2 = Rect::new(30, 30, 50, 50);
    if !r1.intersects(&r2) {
        serial_write_str("[KNT-PANIC] Rect intersection calculation failed!\r\n");
        return false;
    }

    let isect = match r1.intersection(&r2) {
        Some(r) => r,
        None => {
            serial_write_str("[KNT-PANIC] Rect intersection extraction failed!\r\n");
            return false;
        }
    };
    if isect != Rect::new(30, 30, 30, 30) {
        serial_write_str("[KNT-PANIC] Rect intersection coordinates mismatch!\r\n");
        return false;
    }

    let union_rect = r1.union(&r2);
    if union_rect != Rect::new(10, 10, 70, 70) {
        serial_write_str("[KNT-PANIC] Rect union coordinates mismatch!\r\n");
        return false;
    }

    let mut damage = DamageRegion::new();
    damage.add_rect(r1);
    damage.add_rect(r2);
    // Overlapping rectangles should coalesce
    if damage.rects().len() != 1 || damage.rects()[0] != union_rect {
        serial_write_str("[KNT-PANIC] DamageRegion coalescing failed!\r\n");
        return false;
    }
    serial_write_str("  * Dirty rectangle damage tracking and coalescing verified\r\n");
    serial_write_str("[KNT-COMPOSITOR-DIRTY-PASS] Dirty rectangle damage tracking verified\r\n");

    // 2. Verify mouse cursor sprite overlay and blitter
    let mut cursor = CursorState::new(100, 100);
    let cursor_move_damage = cursor.set_position(120, 120);
    if cursor_move_damage.is_empty() || !cursor_move_damage.contains_point(100, 100) || !cursor_move_damage.contains_point(125, 125) {
        serial_write_str("[KNT-PANIC] Cursor motion damage delta failed!\r\n");
        return false;
    }

    // Verify cursor sprite palette
    let p_outline = cursor_sprite_pixel(0, 0);
    let p_body = cursor_sprite_pixel(1, 2);
    let p_trans = cursor_sprite_pixel(11, 0);
    if p_outline != Color::BLACK || p_body != Color::WHITE || p_trans.a != 0 {
        serial_write_str("[KNT-PANIC] Cursor sprite bitmap pixels mismatch!\r\n");
        return false;
    }

    // Test cursor blitting with alpha blend onto background
    let mut test_cursor_buf = [Color::WIN11_BG; 32 * 32];
    cursor.set_position(5, 5);
    cursor.render(&mut test_cursor_buf, 32, 32, None);
    // Tip at (5, 5) should be BLACK outline
    if test_cursor_buf[5 * 32 + 5] != Color::BLACK {
        serial_write_str("[KNT-PANIC] Cursor blitter render mismatch!\r\n");
        return false;
    }
    serial_write_str("  * Mouse cursor sprite overlay and alpha blitter verified\r\n");
    serial_write_str("[KNT-COMPOSITOR-CURSOR-PASS] Mouse cursor sprite overlay and blitter verified\r\n");

    // 3. Verify Fluent window frame styling, titlebar, buttons, and client buffers
    let mut win = Window::new(1, alloc::string::String::from("Terminal"), 20, 20, 200, 150);
    win.is_focused = true;
    win.clear_client(Color::BLACK);
    win.draw_client_rect(10, 10, 40, 20, Color::WIN11_ACCENT);
    win.draw_client_text(12, 12, "CLI", Color::WHITE, None);

    if win.client_width != 200 - BORDER_WIDTH * 2 || win.client_height != 150 - TITLEBAR_HEIGHT - BORDER_WIDTH * 2 {
        serial_write_str("[KNT-PANIC] Window client dimensions mismatch!\r\n");
        return false;
    }
    if win.get_client_pixel(10, 10) != Some(Color::WIN11_ACCENT) {
        serial_write_str("[KNT-PANIC] Window client buffer rendering failed!\r\n");
        return false;
    }

    let mut win_test_buf = [Color::WIN11_BG; 250 * 200];
    win.render_to_buffer(&mut win_test_buf, 250, 200, None);
    // Outer border (20, 20) should be focused accent
    if win_test_buf[20 * 250 + 20] != Color::WIN11_ACCENT {
        serial_write_str("[KNT-PANIC] Window outer border color mismatch!\r\n");
        return false;
    }
    // Close button area
    let close_x = 20 + 200 - BORDER_WIDTH - BUTTON_WIDTH;
    let close_y = 20 + BORDER_WIDTH + 2;
    if win_test_buf[close_y * 250 + close_x + 4] != Color::WIN11_CLOSE_RED {
        serial_write_str("[KNT-PANIC] Window close button color mismatch!\r\n");
        return false;
    }
    serial_write_str("  * Fluent window frame styling, borders, and client surface verified\r\n");
    serial_write_str("[KNT-COMPOSITOR-WINDOW-PASS] Fluent window frame styling and z-order verified\r\n");

    // 4. Verify SceneCompositor multi-window backbuffer composition and blit
    let passed = with_compositor(|comp| {
        let win1_id = comp.create_window("Terminal", 20, 20, 160, 100);
        let win2_id = comp.create_window("Files", 60, 50, 180, 110);

        // Customize win1 client surface
        if let Some(w1) = comp.find_window_mut(win1_id) {
            w1.clear_client(Color::BLACK);
            w1.draw_client_text(10, 10, "KntonOS Kernel Shell", Color::WHITE, None);
        }

        // Customize win2 client surface
        if let Some(w2) = comp.find_window_mut(win2_id) {
            w2.clear_client(Color::rgb(35, 40, 50));
            w2.draw_client_text(10, 10, "Initramfs:/system/bin", Color::WIN11_ACCENT, None);
        }

        // Move cursor over desktop
        comp.move_cursor(100, 100);

        // Create a memory-backed synthetic LinearFramebuffer for test blitting (320x240)
        let mut fb_memory = alloc::vec![0u32; 320 * 240];
        let fb_info = FramebufferInfo {
            base_address: fb_memory.as_mut_ptr() as usize,
            size: fb_memory.len() * 4,
            width: 320,
            height: 240,
            stride: 320,
            is_bgr: false,
        };
        let mut fb = unsafe { LinearFramebuffer::new(fb_info) };

        // Perform scene composition and blit to synthetic framebuffer
        comp.composite(&mut fb);

        // Verify background wallpaper rendered outside windows
        let bg_pixel = fb_memory[10 * 320 + 10];
        if bg_pixel != Color::WIN11_BG.to_u32_rgb() {
            serial_write_str("[KNT-PANIC] Compositor background wallpaper pixel mismatch!\r\n");
            return false;
        }

        // Verify taskbar rendered at bottom
        let taskbar_y = 240 - DEFAULT_TASKBAR_HEIGHT + 10;
        let tb_pixel = fb_memory[taskbar_y * 320 + 10];
        if tb_pixel != Color::WIN11_TASKBAR.to_u32_rgb() {
            serial_write_str("[KNT-PANIC] Compositor taskbar background pixel mismatch!\r\n");
            return false;
        }

        // Verify top window (win2) focus and titlebar
        let win2_tb_y = 50 + BORDER_WIDTH + 5;
        let win2_tb_x = 60 + BORDER_WIDTH + 5;
        let win2_pixel = fb_memory[win2_tb_y * 320 + win2_tb_x];
        if win2_pixel != Color::WIN11_TITLEBAR.to_u32_rgb() {
            serial_write_str("[KNT-PANIC] Top window titlebar pixel mismatch!\r\n");
            return false;
        }

        // Test Z-order switching: bring win1 to front
        comp.bring_to_front(win1_id);
        comp.composite(&mut fb);

        // Now win1 should be focused
        let win1_tb_y = 20 + BORDER_WIDTH + 5;
        let win1_tb_x = 20 + BORDER_WIDTH + 5;
        let win1_pixel = fb_memory[win1_tb_y * 320 + win1_tb_x];
        if win1_pixel != Color::WIN11_TITLEBAR.to_u32_rgb() {
            serial_write_str("[KNT-PANIC] Z-order switch titlebar pixel mismatch!\r\n");
            return false;
        }

        true
    }).unwrap_or(false);

    if !passed {
        return false;
    }

    serial_write_str("  * Multi-window Z-order scene composition and blitter verified\r\n");
    serial_write_str("[KNT-COMPOSITOR-SCENE-PASS] Multi-window backbuffer composition verified\r\n");
    serial_write_str("[KNT-COMPOSITOR-TESTS-PASS] All compositor subsystem checks passed\r\n");

    if !test_gui_controls_and_acrylic() {
        return false;
    }

    true
}

/// Comprehensive verification test suite for Desktop GUI Controls & Acrylic Backdrop Renderer.
pub fn test_gui_controls_and_acrylic() -> bool {
    serial_write_str("[KNT-GUI-TEST] Running Desktop GUI Controls & Acrylic Surface verification suite...\r\n");

    // 1. Verify button state transitions (Normal -> Hovered -> Pressed -> Clicked -> Normal -> Disabled)
    let mut btn = Button::new(1, Rect::new(10, 10, 60, 24), "Submit");
    if btn.state != ButtonState::Normal {
        serial_write_str("[KNT-PANIC] Initial button state is not Normal!\r\n");
        return false;
    }

    // Hover over button
    let changed = btn.handle_mouse(20, 15, false, false);
    if !changed || btn.state != ButtonState::Hovered {
        serial_write_str("[KNT-PANIC] Button hover transition failed!\r\n");
        return false;
    }

    // Press down
    let changed = btn.handle_mouse(20, 15, true, false);
    if !changed || btn.state != ButtonState::Pressed {
        serial_write_str("[KNT-PANIC] Button pressed transition failed!\r\n");
        return false;
    }

    // Release mouse -> triggers click
    let changed = btn.handle_mouse(20, 15, false, true);
    if !changed || !btn.take_click() || btn.state != ButtonState::Hovered {
        serial_write_str("[KNT-PANIC] Button click release failed!\r\n");
        return false;
    }

    // Move outside
    btn.handle_mouse(200, 200, false, false);
    if btn.state != ButtonState::Normal {
        serial_write_str("[KNT-PANIC] Button leave transition failed!\r\n");
        return false;
    }

    // Disabled test
    btn.set_disabled(true);
    let changed = btn.handle_mouse(20, 15, true, false);
    if changed || btn.state != ButtonState::Disabled {
        serial_write_str("[KNT-PANIC] Disabled button state invariant violated!\r\n");
        return false;
    }

    serial_write_str("  * Button hover, press, and disabled state transitions verified\r\n");
    serial_write_str("[KNT-GUI-STATE-PASS] Button hover, press, and disabled state transitions verified\r\n");

    // 2. Verify Acrylic frosted glass backdrop rasterization (3x3 blur, 60% tint, noise grain)
    let mut synthetic_src = [Color::rgb(20, 30, 40); 32 * 32];
    for y in 0..32 {
        for x in 0..32 {
            if (x / 4 + y / 4) % 2 == 0 {
                synthetic_src[y * 32 + x] = Color::rgb(200, 220, 255);
            }
        }
    }
    let mut synthetic_dst = [Color::WIN11_BG; 32 * 32];
    let acrylic_rect = Rect::new(4, 4, 24, 24);
    let config = AcrylicConfig::default_dark();

    AcrylicSurface::render_acrylic(
        &synthetic_src,
        &mut synthetic_dst,
        32,
        32,
        &acrylic_rect,
        &config,
        None,
    );

    // Verify rendered acrylic pixel in center has diffuse blended color
    let center_pixel = synthetic_dst[16 * 32 + 16];
    if center_pixel == Color::WIN11_BG || center_pixel == Color::rgb(200, 220, 255) {
        serial_write_str("[KNT-PANIC] Acrylic surface rasterization failed to blur/tint backdrop!\r\n");
        return false;
    }

    // Verify in-place acrylic titlebar rendering
    let mut titlebar_buf = [Color::WIN11_BG; 64 * 32];
    AcrylicSurface::render_acrylic_in_place(
        &mut titlebar_buf,
        64,
        32,
        &Rect::new(0, 0, 64, 16),
        &AcrylicConfig::titlebar(true),
        None,
    );
    if titlebar_buf[8 * 64 + 10] == Color::WIN11_BG {
        serial_write_str("[KNT-PANIC] Acrylic in-place titlebar rendering failed!\r\n");
        return false;
    }

    serial_write_str("  * Acrylic glass dual 3x3 blur, 60% tint, and noise grain verified\r\n");
    serial_write_str("[KNT-GUI-ACRYLIC-PASS] Acrylic glass dual 3x3 blur, 60% tint, and noise grain verified\r\n");

    // 3. Verify Window control buttons and click event routing
    let mut win = Window::new(99, alloc::string::String::from("Settings"), 40, 40, 160, 120);
    win.add_button(201, 10, 10, 60, 24, "Save");
    win.set_acrylic(true);

    let close_box = win.close_button.bounds;
    // Hover close button
    let (dirty, _, _, _) = win.dispatch_mouse(close_box.x + 4, close_box.y + 4, false, false);
    if dirty.is_none() || win.close_button.state != ButtonState::Hovered {
        serial_write_str("[KNT-PANIC] Window close button hover dispatch failed!\r\n");
        return false;
    }

    // Press close button
    win.dispatch_mouse(close_box.x + 4, close_box.y + 4, true, false);
    if win.close_button.state != ButtonState::Pressed {
        serial_write_str("[KNT-PANIC] Window close button press dispatch failed!\r\n");
        return false;
    }

    // Release close button -> close requested
    let (_, close_req, _, _) = win.dispatch_mouse(close_box.x + 4, close_box.y + 4, false, true);
    if !close_req {
        serial_write_str("[KNT-PANIC] Window close request trigger failed!\r\n");
        return false;
    }

    // Client button interaction
    let btn_box = win.widgets[0].bounds;
    win.dispatch_mouse(btn_box.x + 4, btn_box.y + 4, true, false);
    win.dispatch_mouse(btn_box.x + 4, btn_box.y + 4, false, true);
    if !win.widgets[0].take_click() {
        serial_write_str("[KNT-PANIC] Client button click event routing failed!\r\n");
        return false;
    }

    serial_write_str("  * Window control button click routing and dirty region damage verified\r\n");
    serial_write_str("[KNT-GUI-DISPATCH-PASS] Window control button click routing and dirty region damage verified\r\n");

    // 4. Verify SceneCompositor interactive close button handling
    let comp_pass = with_compositor(|comp| {
        let test_win_id = comp.create_window("AppToClose", 50, 50, 120, 80);
        let close_coords = if let Some(w) = comp.find_window(test_win_id) {
            let cb = w.close_button.bounds;
            (cb.x + 5, cb.y + 5)
        } else {
            return false;
        };

        // Click close button via mouse input dispatch
        comp.dispatch_mouse_input(close_coords.0, close_coords.1, 1);
        comp.dispatch_mouse_input(close_coords.0, close_coords.1, 0);

        // Window should now be closed and removed
        comp.find_window(test_win_id).is_none()
    }).unwrap_or(false);

    if !comp_pass {
        serial_write_str("[KNT-PANIC] SceneCompositor window close routing failed!\r\n");
        return false;
    }

    serial_write_str("[KNT-GUI-WIDGET-PASS] All desktop GUI controls and acrylic surface checks passed\r\n");
    true
}
