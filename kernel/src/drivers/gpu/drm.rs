//! Direct Rendering Manager (DRM) and Kernel Mode Setting (KMS) abstractions.

// DRM Mode Object Types.

pub const DRM_MODE_OBJECT_CRTC: u32 = 0xCCCC_CCCC;
pub const DRM_MODE_OBJECT_CONNECTOR: u32 = 0xC0C0_C0C0;
pub const DRM_MODE_OBJECT_ENCODER: u32 = 0xEEEE_EEEE;
pub const DRM_MODE_OBJECT_MODE: u32 = 0xDEDE_DEDE;
pub const DRM_MODE_OBJECT_PROPERTY: u32 = 0xB0B0_B0B0;
pub const DRM_MODE_OBJECT_FB: u32 = 0xFBFB_FBFB;
pub const DRM_MODE_OBJECT_BLOB: u32 = 0xBBBB_BBBB;
pub const DRM_MODE_OBJECT_PLANE: u32 = 0xEEEE_0001;

// DRM Capabilities.

pub const DRM_CAP_DUMB_BUFFER: u64 = 0x1;
pub const DRM_CAP_VBLANK_HIGH_CRTC: u64 = 0x2;
pub const DRM_CAP_CRTC_IN_VBLANK_EVENT: u64 = 0x3;
pub const DRM_CAP_TIMESTAMP_MONOTONIC: u64 = 0x6;
pub const DRM_CAP_PRIME: u64 = 0x7;
pub const DRM_CAP_ADDFB2_MODIFIERS: u64 = 0x10;
pub const DRM_CAP_ATOMIC: u64 = 0x11;

// DRM Format FourCC Codes (Little-Endian).

pub const DRM_FORMAT_INVALID: u32 = 0;
pub const DRM_FORMAT_XRGB8888: u32 = 0x3432_5258; // 'X' 'R' '2' '4'
pub const DRM_FORMAT_ARGB8888: u32 = 0x3432_5241; // 'A' 'R' '2' '4'
pub const DRM_FORMAT_XBGR8888: u32 = 0x3432_4258; // 'X' 'B' '2' '4'
pub const DRM_FORMAT_ABGR8888: u32 = 0x3432_4241; // 'A' 'B' '2' '4'
pub const DRM_FORMAT_RGB565: u32 = 0x3631_4752;   // 'R' 'G' '1' '6'

// Plane Types & Capabilities.

pub const DRM_PLANE_TYPE_OVERLAY: u32 = 0;
pub const DRM_PLANE_TYPE_PRIMARY: u32 = 1;
pub const DRM_PLANE_TYPE_CURSOR: u32 = 2;

pub const DRM_MODE_ROTATE_0: u32 = 1 << 0;
pub const DRM_MODE_ROTATE_90: u32 = 1 << 1;
pub const DRM_MODE_ROTATE_180: u32 = 1 << 2;
pub const DRM_MODE_ROTATE_270: u32 = 1 << 3;
pub const DRM_MODE_REFLECT_X: u32 = 1 << 4;
pub const DRM_MODE_REFLECT_Y: u32 = 1 << 5;
pub const DRM_MODE_ROTATE_MASK: u32 = DRM_MODE_ROTATE_0 | DRM_MODE_ROTATE_90 | DRM_MODE_ROTATE_180 | DRM_MODE_ROTATE_270;
pub const DRM_MODE_REFLECT_MASK: u32 = DRM_MODE_REFLECT_X | DRM_MODE_REFLECT_Y;

// DRM Atomic Properties & Flags.

pub const DRM_MODE_ATOMIC_TEST_ONLY: u32 = 0x0100;
pub const DRM_MODE_ATOMIC_NONBLOCK: u32 = 0x0200;
pub const DRM_MODE_ATOMIC_ALLOW_MODESET: u32 = 0x0400;

pub const DRM_PROP_FB_ID: u32 = 1;
pub const DRM_PROP_CRTC_ID: u32 = 2;
pub const DRM_PROP_CRTC_X: u32 = 3;
pub const DRM_PROP_CRTC_Y: u32 = 4;
pub const DRM_PROP_CRTC_W: u32 = 5;
pub const DRM_PROP_CRTC_H: u32 = 6;
pub const DRM_PROP_SRC_X: u32 = 7;
pub const DRM_PROP_SRC_Y: u32 = 8;
pub const DRM_PROP_SRC_W: u32 = 9;
pub const DRM_PROP_SRC_H: u32 = 10;
pub const DRM_PROP_ROTATION: u32 = 11;
pub const DRM_PROP_SCALING_FILTER: u32 = 12;
pub const DRM_PROP_ACTIVE: u32 = 13;
pub const DRM_PROP_MODE_ID: u32 = 14;
pub const DRM_PROP_ALPHA: u32 = 15;
pub const DRM_PROP_ZPOS: u32 = 16;

pub const DRM_SCALING_FILTER_DEFAULT: u32 = 0;
pub const DRM_SCALING_FILTER_NEAREST: u32 = 1;
pub const DRM_SCALING_FILTER_LINEAR: u32 = 2;

// Encoder Types.

pub const DRM_MODE_ENCODER_NONE: u32 = 0;
pub const DRM_MODE_ENCODER_DAC: u32 = 1;
pub const DRM_MODE_ENCODER_TMDS: u32 = 2;
pub const DRM_MODE_ENCODER_LVDS: u32 = 3;
pub const DRM_MODE_ENCODER_TVDAC: u32 = 4;
pub const DRM_MODE_ENCODER_VIRTUAL: u32 = 5;
pub const DRM_MODE_ENCODER_DSI: u32 = 6;
pub const DRM_MODE_ENCODER_DPMST: u32 = 7;
pub const DRM_MODE_ENCODER_DPI: u32 = 8;

// Connector Types & Connection Status.

pub const DRM_MODE_CONNECTOR_UNKNOWN: u32 = 0;
pub const DRM_MODE_CONNECTOR_VGA: u32 = 1;
pub const DRM_MODE_CONNECTOR_DVII: u32 = 2;
pub const DRM_MODE_CONNECTOR_DVID: u32 = 3;
pub const DRM_MODE_CONNECTOR_DVIA: u32 = 4;
pub const DRM_MODE_CONNECTOR_COMPOSITE: u32 = 5;
pub const DRM_MODE_CONNECTOR_SVIDEO: u32 = 6;
pub const DRM_MODE_CONNECTOR_LVDS: u32 = 7;
pub const DRM_MODE_CONNECTOR_COMPONENT: u32 = 8;
pub const DRM_MODE_CONNECTOR_9PINDIN: u32 = 9;
pub const DRM_MODE_CONNECTOR_DISPLAYPORT: u32 = 10;
pub const DRM_MODE_CONNECTOR_HDMIA: u32 = 11;
pub const DRM_MODE_CONNECTOR_HDMIB: u32 = 12;
pub const DRM_MODE_CONNECTOR_TV: u32 = 13;
pub const DRM_MODE_CONNECTOR_EDP: u32 = 14;
pub const DRM_MODE_CONNECTOR_VIRTUAL: u32 = 15;
pub const DRM_MODE_CONNECTOR_DSI: u32 = 16;
pub const DRM_MODE_CONNECTOR_DPI: u32 = 17;

pub const DRM_MODE_CONNECTED: u32 = 1;
pub const DRM_MODE_DISCONNECTED: u32 = 2;
pub const DRM_MODE_UNKNOWNCONNECTION: u32 = 3;

pub const DRM_MODE_SUBPIXEL_UNKNOWN: u32 = 1;
pub const DRM_MODE_SUBPIXEL_HORIZONTAL_RGB: u32 = 2;
pub const DRM_MODE_SUBPIXEL_HORIZONTAL_BGR: u32 = 3;
pub const DRM_MODE_SUBPIXEL_VERTICAL_RGB: u32 = 4;
pub const DRM_MODE_SUBPIXEL_VERTICAL_BGR: u32 = 5;
pub const DRM_MODE_SUBPIXEL_NONE: u32 = 6;

// Video Modeline Flags & Types.

pub const DRM_MODE_FLAG_PHSYNC: u32 = 1 << 0;
pub const DRM_MODE_FLAG_NHSYNC: u32 = 1 << 1;
pub const DRM_MODE_FLAG_PVSYNC: u32 = 1 << 2;
pub const DRM_MODE_FLAG_NVSYNC: u32 = 1 << 3;
pub const DRM_MODE_FLAG_INTERLACE: u32 = 1 << 4;
pub const DRM_MODE_FLAG_DBLSCAN: u32 = 1 << 5;
pub const DRM_MODE_FLAG_CSYNC: u32 = 1 << 6;
pub const DRM_MODE_FLAG_PCSYNC: u32 = 1 << 7;
pub const DRM_MODE_FLAG_NCSYNC: u32 = 1 << 8;

pub const DRM_MODE_TYPE_BUILTIN: u32 = 1 << 0;
pub const DRM_MODE_TYPE_CLOCK_C: u32 = 1 << 1;
pub const DRM_MODE_TYPE_CRTC_C: u32 = 1 << 2;
pub const DRM_MODE_TYPE_PREFERRED: u32 = 1 << 3;
pub const DRM_MODE_TYPE_DEFAULT: u32 = 1 << 4;
pub const DRM_MODE_TYPE_USERDEF: u32 = 1 << 5;
pub const DRM_MODE_TYPE_DRIVER: u32 = 1 << 6;

// DRM Mode Info (Modeline / Timing Structure).

/// Standard DRM Video Modeline descriptor.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrmModeInfo {
    pub clock: u32,       // Pixel clock in kHz
    pub hdisplay: u16,    // Horizontal active resolution
    pub hsync_start: u16, // Horizontal sync pulse start
    pub hsync_end: u16,   // Horizontal sync pulse end
    pub htotal: u16,      // Horizontal total timing (active + porch + sync)
    pub hskew: u16,
    pub vdisplay: u16,    // Vertical active resolution
    pub vsync_start: u16, // Vertical sync pulse start
    pub vsync_end: u16,   // Vertical sync pulse end
    pub vtotal: u16,      // Vertical total timing
    pub vscan: u16,
    pub vrefresh: u32,    // Vertical refresh rate in Hz
    pub flags: u32,       // Sync polarity and scan flags
    pub mode_type: u32,   // Mode classification (preferred, driver, builtin)
    pub name: [u8; 32],   // Human-readable identifier (e.g. "1920x1080")
}

impl DrmModeInfo {
    pub const fn empty() -> Self {
        Self {
            clock: 0,
            hdisplay: 0,
            hsync_start: 0,
            hsync_end: 0,
            htotal: 0,
            hskew: 0,
            vdisplay: 0,
            vsync_start: 0,
            vsync_end: 0,
            vtotal: 0,
            vscan: 0,
            vrefresh: 0,
            flags: 0,
            mode_type: 0,
            name: [0; 32],
        }
    }

    pub fn new(
        name_str: &str,
        clock: u32,
        hdisplay: u16,
        hsync_start: u16,
        hsync_end: u16,
        htotal: u16,
        vdisplay: u16,
        vsync_start: u16,
        vsync_end: u16,
        vtotal: u16,
        vrefresh: u32,
        flags: u32,
        mode_type: u32,
    ) -> Self {
        let mut name = [0u8; 32];
        let bytes = name_str.as_bytes();
        let copy_len = if bytes.len() > 31 { 31 } else { bytes.len() };
        let mut i = 0;
        while i < copy_len {
            name[i] = bytes[i];
            i += 1;
        }

        Self {
            clock,
            hdisplay,
            hsync_start,
            hsync_end,
            htotal,
            hskew: 0,
            vdisplay,
            vsync_start,
            vsync_end,
            vtotal,
            vscan: 0,
            vrefresh,
            flags,
            mode_type,
            name,
        }
    }

    pub fn name(&self) -> &str {
        let mut len = 0;
        while len < self.name.len() && self.name[len] != 0 {
            len += 1;
        }
        core::str::from_utf8(&self.name[..len]).unwrap_or("unknown")
    }

    #[inline(always)]
    pub fn hblank(&self) -> u16 {
        self.htotal.saturating_sub(self.hdisplay)
    }

    #[inline(always)]
    pub fn vblank(&self) -> u16 {
        self.vtotal.saturating_sub(self.vdisplay)
    }

    #[inline(always)]
    pub fn bandwidth_bps(&self) -> u64 {
        (self.clock as u64) * 1000 * 32 // 32 bits-per-pixel throughput in bits/sec
    }
}

// Standard VESA/CEA/CTA Video Mode Database.

pub const MODE_640X480_60: DrmModeInfo = DrmModeInfo {
    clock: 25175,
    hdisplay: 640,
    hsync_start: 656,
    hsync_end: 752,
    htotal: 800,
    hskew: 0,
    vdisplay: 480,
    vsync_start: 490,
    vsync_end: 492,
    vtotal: 525,
    vscan: 0,
    vrefresh: 60,
    flags: DRM_MODE_FLAG_NHSYNC | DRM_MODE_FLAG_NVSYNC,
    mode_type: DRM_MODE_TYPE_DRIVER | DRM_MODE_TYPE_DEFAULT,
    name: *b"640x480\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
};

pub const MODE_800X600_60: DrmModeInfo = DrmModeInfo {
    clock: 40000,
    hdisplay: 800,
    hsync_start: 840,
    hsync_end: 968,
    htotal: 1056,
    hskew: 0,
    vdisplay: 600,
    vsync_start: 601,
    vsync_end: 605,
    vtotal: 628,
    vscan: 0,
    vrefresh: 60,
    flags: DRM_MODE_FLAG_PHSYNC | DRM_MODE_FLAG_PVSYNC,
    mode_type: DRM_MODE_TYPE_DRIVER,
    name: *b"800x600\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
};

pub const MODE_1024X768_60: DrmModeInfo = DrmModeInfo {
    clock: 65000,
    hdisplay: 1024,
    hsync_start: 1048,
    hsync_end: 1184,
    htotal: 1344,
    hskew: 0,
    vdisplay: 768,
    vsync_start: 771,
    vsync_end: 777,
    vtotal: 806,
    vscan: 0,
    vrefresh: 60,
    flags: DRM_MODE_FLAG_NHSYNC | DRM_MODE_FLAG_NVSYNC,
    mode_type: DRM_MODE_TYPE_DRIVER,
    name: *b"1024x768\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
};

pub const MODE_1280X720_60: DrmModeInfo = DrmModeInfo {
    clock: 74250,
    hdisplay: 1280,
    hsync_start: 1390,
    hsync_end: 1430,
    htotal: 1650,
    hskew: 0,
    vdisplay: 720,
    vsync_start: 725,
    vsync_end: 730,
    vtotal: 750,
    vscan: 0,
    vrefresh: 60,
    flags: DRM_MODE_FLAG_PHSYNC | DRM_MODE_FLAG_PVSYNC,
    mode_type: DRM_MODE_TYPE_DRIVER,
    name: *b"1280x720\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
};

pub const MODE_1920X1080_60: DrmModeInfo = DrmModeInfo {
    clock: 148500,
    hdisplay: 1920,
    hsync_start: 2008,
    hsync_end: 2052,
    htotal: 2200,
    hskew: 0,
    vdisplay: 1080,
    vsync_start: 1084,
    vsync_end: 1089,
    vtotal: 1125,
    vscan: 0,
    vrefresh: 60,
    flags: DRM_MODE_FLAG_PHSYNC | DRM_MODE_FLAG_PVSYNC,
    mode_type: DRM_MODE_TYPE_DRIVER | DRM_MODE_TYPE_PREFERRED,
    name: *b"1920x1080\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
};

pub const MODE_2560X1440_60: DrmModeInfo = DrmModeInfo {
    clock: 241500,
    hdisplay: 2560,
    hsync_start: 2608,
    hsync_end: 2640,
    htotal: 2720,
    hskew: 0,
    vdisplay: 1440,
    vsync_start: 1443,
    vsync_end: 1448,
    vtotal: 1481,
    vscan: 0,
    vrefresh: 60,
    flags: DRM_MODE_FLAG_PHSYNC | DRM_MODE_FLAG_NVSYNC,
    mode_type: DRM_MODE_TYPE_DRIVER,
    name: *b"2560x1440\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
};

pub const MODE_3840X2160_60: DrmModeInfo = DrmModeInfo {
    clock: 594000,
    hdisplay: 3840,
    hsync_start: 4016,
    hsync_end: 4104,
    htotal: 4400,
    hskew: 0,
    vdisplay: 2160,
    vsync_start: 2168,
    vsync_end: 2178,
    vtotal: 2250,
    vscan: 0,
    vrefresh: 60,
    flags: DRM_MODE_FLAG_PHSYNC | DRM_MODE_FLAG_PVSYNC,
    mode_type: DRM_MODE_TYPE_DRIVER,
    name: *b"3840x2160\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
};

pub static STANDARD_VIDEO_MODES: [DrmModeInfo; 7] = [
    MODE_640X480_60,
    MODE_800X600_60,
    MODE_1024X768_60,
    MODE_1280X720_60,
    MODE_1920X1080_60,
    MODE_2560X1440_60,
    MODE_3840X2160_60,
];

pub fn find_mode_by_resolution(width: u16, height: u16, refresh: u32) -> Option<DrmModeInfo> {
    for mode in STANDARD_VIDEO_MODES.iter() {
        if mode.hdisplay == width && mode.vdisplay == height && (refresh == 0 || mode.vrefresh == refresh) {
            return Some(*mode);
        }
    }
    None
}

// DRM CRTC (Scanout Engine & Timing Controller).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrmCrtc {
    pub id: u32,
    pub index: usize,
    pub enabled: bool,
    pub active: bool,
    pub mode: Option<DrmModeInfo>,
    pub gamma_lut_size: u32,
    pub current_fb_id: u32,
    pub x: u32,
    pub y: u32,
    pub vblank_counter: u64,
}

impl DrmCrtc {
    pub const fn new(id: u32, index: usize) -> Self {
        Self {
            id,
            index,
            enabled: false,
            active: false,
            mode: None,
            gamma_lut_size: 256,
            current_fb_id: 0,
            x: 0,
            y: 0,
            vblank_counter: 0,
        }
    }

    pub fn set_mode(&mut self, mode: DrmModeInfo) {
        self.mode = Some(mode);
        self.enabled = true;
        self.active = true;
    }

    pub fn disable(&mut self) {
        self.active = false;
        self.enabled = false;
        self.current_fb_id = 0;
    }

    pub fn on_vblank(&mut self) {
        self.vblank_counter = self.vblank_counter.wrapping_add(1);
    }
}

// DRM Plane (Primary, Cursor, Hardware Overlay).

pub const MAX_PLANE_FORMATS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrmPlane {
    pub id: u32,
    pub plane_type: u32,
    pub possible_crtcs: u32,
    pub formats: [u32; MAX_PLANE_FORMATS],
    pub format_count: usize,
    pub fb_id: u32,
    pub crtc_id: u32,
    pub crtc_x: i32,
    pub crtc_y: i32,
    pub crtc_w: u32,
    pub crtc_h: u32,
    pub src_x: u32, // 16.16 fixed point (or pixel << 16)
    pub src_y: u32,
    pub src_w: u32,
    pub src_h: u32,
    pub zpos: u32,
    pub alpha: u16, // 0xFFFF = fully opaque, 0 = transparent
    pub rotation: u32,
    pub scaling_filter: u32,
}

impl DrmPlane {
    pub const fn new(id: u32, plane_type: u32, possible_crtcs: u32) -> Self {
        let (zpos, alpha) = match plane_type {
            DRM_PLANE_TYPE_PRIMARY => (0, 0xFFFF),
            DRM_PLANE_TYPE_OVERLAY => (1, 0xFFFF),
            DRM_PLANE_TYPE_CURSOR => (2, 0xFFFF),
            _ => (0, 0xFFFF),
        };

        Self {
            id,
            plane_type,
            possible_crtcs,
            formats: [DRM_FORMAT_INVALID; MAX_PLANE_FORMATS],
            format_count: 0,
            fb_id: 0,
            crtc_id: 0,
            crtc_x: 0,
            crtc_y: 0,
            crtc_w: 0,
            crtc_h: 0,
            src_x: 0,
            src_y: 0,
            src_w: 0,
            src_h: 0,
            zpos,
            alpha,
            rotation: DRM_MODE_ROTATE_0,
            scaling_filter: DRM_SCALING_FILTER_DEFAULT,
        }
    }

    pub fn add_format(&mut self, format: u32) -> bool {
        if self.format_count < MAX_PLANE_FORMATS {
            self.formats[self.format_count] = format;
            self.format_count += 1;
            true
        } else {
            false
        }
    }

    pub fn supports_format(&self, format: u32) -> bool {
        for i in 0..self.format_count {
            if self.formats[i] == format {
                return true;
            }
        }
        false
    }

    pub fn is_compatible_crtc(&self, crtc_index: usize) -> bool {
        (self.possible_crtcs & (1 << crtc_index)) != 0
    }
}

// DRM Encoder (Pixel Serializer / Bridge).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrmEncoder {
    pub id: u32,
    pub encoder_type: u32,
    pub possible_crtcs: u32,
    pub possible_clones: u32,
    pub crtc_id: u32,
}

impl DrmEncoder {
    pub const fn new(id: u32, encoder_type: u32, possible_crtcs: u32) -> Self {
        Self {
            id,
            encoder_type,
            possible_crtcs,
            possible_clones: 0,
            crtc_id: 0,
        }
    }

    pub fn is_compatible_crtc(&self, crtc_index: usize) -> bool {
        (self.possible_crtcs & (1 << crtc_index)) != 0
    }
}

// DRM Connector (Physical/Virtual Display Output).

pub const MAX_CONNECTOR_MODES: usize = 8;
pub const MAX_CONNECTOR_ENCODERS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrmConnector {
    pub id: u32,
    pub connector_type: u32,
    pub connection_status: u32,
    pub encoder_id: u32,
    pub possible_encoders: u32, // Bitmask of compatible encoders
    pub mm_width: u32,
    pub mm_height: u32,
    pub subpixel: u32,
    pub modes: [DrmModeInfo; MAX_CONNECTOR_MODES],
    pub mode_count: usize,
}

impl DrmConnector {
    pub const fn new(id: u32, connector_type: u32, possible_encoders: u32) -> Self {
        Self {
            id,
            connector_type,
            connection_status: DRM_MODE_DISCONNECTED,
            encoder_id: 0,
            possible_encoders,
            mm_width: 0,
            mm_height: 0,
            subpixel: DRM_MODE_SUBPIXEL_HORIZONTAL_RGB,
            modes: [DrmModeInfo::empty(); MAX_CONNECTOR_MODES],
            mode_count: 0,
        }
    }

    pub fn add_mode(&mut self, mode: DrmModeInfo) -> bool {
        if self.mode_count < MAX_CONNECTOR_MODES {
            self.modes[self.mode_count] = mode;
            self.mode_count += 1;
            true
        } else {
            false
        }
    }

    pub fn set_connected(&mut self, connected: bool) {
        self.connection_status = if connected {
            DRM_MODE_CONNECTED
        } else {
            DRM_MODE_DISCONNECTED
        };
    }

    pub fn is_connected(&self) -> bool {
        self.connection_status == DRM_MODE_CONNECTED
    }

    pub fn preferred_mode(&self) -> Option<DrmModeInfo> {
        for i in 0..self.mode_count {
            if (self.modes[i].mode_type & DRM_MODE_TYPE_PREFERRED) != 0 {
                return Some(self.modes[i]);
            }
        }
        if self.mode_count > 0 {
            Some(self.modes[0])
        } else {
            None
        }
    }
}

// DRM Framebuffer Abstraction.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrmFramebuffer {
    pub id: u32,
    pub width: u32,
    pub height: u32,
    pub pixel_format: u32,
    pub pitches: [u32; 4],
    pub offsets: [u32; 4],
    pub modifier: u64,
    pub phys_addr: u64,
    pub virt_addr: usize,
}

impl DrmFramebuffer {
    pub const fn new(
        id: u32,
        width: u32,
        height: u32,
        pixel_format: u32,
        pitch: u32,
        phys_addr: u64,
        virt_addr: usize,
    ) -> Self {
        Self {
            id,
            width,
            height,
            pixel_format,
            pitches: [pitch, 0, 0, 0],
            offsets: [0, 0, 0, 0],
            modifier: 0,
            phys_addr,
            virt_addr,
        }
    }

    pub fn size_bytes(&self) -> usize {
        (self.pitches[0] as usize) * (self.height as usize)
    }
}

// DRM Device (KMS Pipeline Topology Container).

pub const MAX_DRM_CRTCS: usize = 2;
pub const MAX_DRM_PLANES: usize = 6;
pub const MAX_DRM_ENCODERS: usize = 2;
pub const MAX_DRM_CONNECTORS: usize = 2;
pub const MAX_DRM_FRAMEBUFFERS: usize = 8;

pub struct DrmDevice {
    pub crtcs: [Option<DrmCrtc>; MAX_DRM_CRTCS],
    pub planes: [Option<DrmPlane>; MAX_DRM_PLANES],
    pub encoders: [Option<DrmEncoder>; MAX_DRM_ENCODERS],
    pub connectors: [Option<DrmConnector>; MAX_DRM_CONNECTORS],
    pub framebuffers: [Option<DrmFramebuffer>; MAX_DRM_FRAMEBUFFERS],
}

impl DrmDevice {
    pub const fn new() -> Self {
        Self {
            crtcs: [None, None],
            planes: [None, None, None, None, None, None],
            encoders: [None, None],
            connectors: [None, None],
            framebuffers: [None, None, None, None, None, None, None, None],
        }
    }

    pub fn register_crtc(&mut self, crtc: DrmCrtc) -> bool {
        for slot in self.crtcs.iter_mut() {
            if slot.is_none() {
                *slot = Some(crtc);
                return true;
            }
        }
        false
    }

    pub fn register_plane(&mut self, plane: DrmPlane) -> bool {
        for slot in self.planes.iter_mut() {
            if slot.is_none() {
                *slot = Some(plane);
                return true;
            }
        }
        false
    }

    pub fn register_encoder(&mut self, encoder: DrmEncoder) -> bool {
        for slot in self.encoders.iter_mut() {
            if slot.is_none() {
                *slot = Some(encoder);
                return true;
            }
        }
        false
    }

    pub fn register_connector(&mut self, connector: DrmConnector) -> bool {
        for slot in self.connectors.iter_mut() {
            if slot.is_none() {
                *slot = Some(connector);
                return true;
            }
        }
        false
    }

    pub fn register_framebuffer(&mut self, fb: DrmFramebuffer) -> bool {
        for slot in self.framebuffers.iter_mut() {
            if slot.is_none() {
                *slot = Some(fb);
                return true;
            }
        }
        false
    }

    pub fn find_crtc(&self, id: u32) -> Option<&DrmCrtc> {
        for slot in self.crtcs.iter() {
            if let Some(ref crtc) = slot {
                if crtc.id == id {
                    return Some(crtc);
                }
            }
        }
        None
    }

    pub fn find_crtc_mut(&mut self, id: u32) -> Option<&mut DrmCrtc> {
        for slot in self.crtcs.iter_mut() {
            if let Some(ref mut crtc) = slot {
                if crtc.id == id {
                    return Some(crtc);
                }
            }
        }
        None
    }

    pub fn find_plane(&self, id: u32) -> Option<&DrmPlane> {
        for slot in self.planes.iter() {
            if let Some(ref plane) = slot {
                if plane.id == id {
                    return Some(plane);
                }
            }
        }
        None
    }

    pub fn find_plane_mut(&mut self, id: u32) -> Option<&mut DrmPlane> {
        for slot in self.planes.iter_mut() {
            if let Some(ref mut plane) = slot {
                if plane.id == id {
                    return Some(plane);
                }
            }
        }
        None
    }

    pub fn find_framebuffer(&self, id: u32) -> Option<&DrmFramebuffer> {
        for slot in self.framebuffers.iter() {
            if let Some(ref fb) = slot {
                if fb.id == id {
                    return Some(fb);
                }
            }
        }
        None
    }
}

// Unit Verification for DRM Abstractions.

/// Verifies DRM object definitions, video mode timings, and topology registration.
pub fn verify_drm_abstractions() -> bool {
    // 1. Verify standard video mode calculations and lookup
    let mode_1080p = find_mode_by_resolution(1920, 1080, 60).expect("1080p mode not found");
    assert_eq!(mode_1080p.hdisplay, 1920);
    assert_eq!(mode_1080p.vdisplay, 1080);
    assert_eq!(mode_1080p.vrefresh, 60);
    assert_eq!(mode_1080p.clock, 148500);
    assert_eq!(mode_1080p.hblank(), 280); // 2200 - 1920
    assert_eq!(mode_1080p.vblank(), 45);  // 1125 - 1080
    assert_eq!(mode_1080p.name(), "1920x1080");

    let mode_720p = find_mode_by_resolution(1280, 720, 60).expect("720p mode not found");
    assert_eq!(mode_720p.hdisplay, 1280);
    assert_eq!(mode_720p.vdisplay, 720);
    assert_eq!(mode_720p.clock, 74250);

    // 2. Verify CRTC state transitions
    let mut crtc = DrmCrtc::new(100, 0);
    assert_eq!(crtc.id, 100);
    assert_eq!(crtc.index, 0);
    assert!(!crtc.active);
    crtc.set_mode(mode_1080p);
    assert!(crtc.active);
    assert!(crtc.enabled);
    assert_eq!(crtc.mode.unwrap().hdisplay, 1920);
    crtc.on_vblank();
    assert_eq!(crtc.vblank_counter, 1);

    // 3. Verify Plane initialization and format registration
    let mut primary_plane = DrmPlane::new(200, DRM_PLANE_TYPE_PRIMARY, 1 << 0);
    assert_eq!(primary_plane.plane_type, DRM_PLANE_TYPE_PRIMARY);
    assert_eq!(primary_plane.zpos, 0);
    assert!(primary_plane.is_compatible_crtc(0));
    assert!(!primary_plane.is_compatible_crtc(1));
    assert!(primary_plane.add_format(DRM_FORMAT_XRGB8888));
    assert!(primary_plane.add_format(DRM_FORMAT_ARGB8888));
    assert!(primary_plane.supports_format(DRM_FORMAT_XRGB8888));
    assert!(primary_plane.supports_format(DRM_FORMAT_ARGB8888));
    assert!(!primary_plane.supports_format(DRM_FORMAT_RGB565));

    let cursor_plane = DrmPlane::new(201, DRM_PLANE_TYPE_CURSOR, 1 << 0);
    assert_eq!(cursor_plane.plane_type, DRM_PLANE_TYPE_CURSOR);
    assert_eq!(cursor_plane.zpos, 2);

    let overlay_plane = DrmPlane::new(202, DRM_PLANE_TYPE_OVERLAY, 1 << 0);
    assert_eq!(overlay_plane.plane_type, DRM_PLANE_TYPE_OVERLAY);
    assert_eq!(overlay_plane.zpos, 1);

    // 4. Verify Connector and Encoder abstractions
    let encoder = DrmEncoder::new(300, DRM_MODE_ENCODER_VIRTUAL, 1 << 0);
    assert_eq!(encoder.encoder_type, DRM_MODE_ENCODER_VIRTUAL);
    assert!(encoder.is_compatible_crtc(0));

    let mut connector = DrmConnector::new(400, DRM_MODE_CONNECTOR_VIRTUAL, 1 << 0);
    assert_eq!(connector.connector_type, DRM_MODE_CONNECTOR_VIRTUAL);
    assert!(!connector.is_connected());
    connector.set_connected(true);
    assert!(connector.is_connected());
    assert!(connector.add_mode(mode_1080p));
    assert!(connector.add_mode(mode_720p));
    assert_eq!(connector.preferred_mode().unwrap().name(), "1920x1080");

    // 5. Verify DRM Device Topology registration
    let mut drm_dev = DrmDevice::new();
    assert!(drm_dev.register_crtc(crtc));
    assert!(drm_dev.register_plane(primary_plane));
    assert!(drm_dev.register_plane(overlay_plane));
    assert!(drm_dev.register_plane(cursor_plane));
    assert!(drm_dev.register_encoder(encoder));
    assert!(drm_dev.register_connector(connector));

    let registered_crtc = drm_dev.find_crtc(100).expect("CRTC 100 lookup failed");
    assert_eq!(registered_crtc.id, 100);

    true
}
