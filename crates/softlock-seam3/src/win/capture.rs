//! Screen captures through GDI, DXGI desktop duplication, and Windows.Graphics.Capture.

use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::{HMODULE, HWND};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_UNKNOWN};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Resource, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO,
    IDXGIAdapter, IDXGIDevice, IDXGIFactory1, IDXGIOutput1, IDXGIOutputDuplication, IDXGIResource,
};
use windows::Win32::Graphics::Gdi::{
    BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, HGDIOBJ, ReleaseDC, SRCCOPY,
    SelectObject,
};
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXSCREEN, SM_CXVIRTUALSCREEN, SM_CYSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};
use windows::core::Interface;

use crate::pixels::{self, MARKER};

use super::error::{HarnessError, explain};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureMethod {
    Gdi,
    DxgiDuplication,
    WindowsGraphicsCapture,
}

impl CaptureMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gdi => "gdi",
            Self::DxgiDuplication => "dxgi",
            Self::WindowsGraphicsCapture => "wgc",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MethodCapture {
    pub method: CaptureMethod,
    pub width: u32,
    pub height: u32,
    pub marker_pixels: u64,
}

struct Frame {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

/// Pixels of `color` in the window's own device context.
///
/// Screen-capture exclusion does not apply to the window DC, so this shows
/// whether the window actually painted.
pub fn window_pixels(
    hwnd: isize,
    color: crate::pixels::Rgb,
    tolerance: u8,
) -> Result<u64, HarnessError> {
    let window = HWND(hwnd as *mut _);
    let mut rect = windows::Win32::Foundation::RECT::default();
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetClientRect(window, &mut rect)
            .map_err(|error| explain(error, "GetClientRect"))?;
    }
    let frame = rect_from_dc(
        Some(window),
        0,
        0,
        rect.right - rect.left,
        rect.bottom - rect.top,
    )?;
    Ok(pixels::count_near(&frame.pixels, color, tolerance))
}

pub fn capture_marker(hwnd: isize) -> Result<Vec<MethodCapture>, HarnessError> {
    capture_near(hwnd, MARKER, 12)
}

/// Counts `color` in one frame from each capture API.
///
/// `MethodCapture::marker_pixels` is the count of `color`, not the probe marker,
/// unless `color` is that marker. Frame selection still prefers the probe marker
/// so a capture that missed the desktop does not look empty.
pub fn capture_near(
    hwnd: isize,
    color: crate::pixels::Rgb,
    tolerance: u8,
) -> Result<Vec<MethodCapture>, HarnessError> {
    let _ = ensure_mta();
    let window = HWND(hwnd as *mut _);
    Ok(vec![
        one(CaptureMethod::Gdi, color, tolerance, || capture_gdi(hwnd))?,
        one(CaptureMethod::DxgiDuplication, color, tolerance, || {
            capture_dxgi(hwnd)
        })?,
        one(
            CaptureMethod::WindowsGraphicsCapture,
            color,
            tolerance,
            || capture_wgc(window, hwnd),
        )?,
    ])
}

fn one(
    method: CaptureMethod,
    color: crate::pixels::Rgb,
    tolerance: u8,
    capture: impl FnOnce() -> Result<Frame, HarnessError>,
) -> Result<MethodCapture, HarnessError> {
    let frame =
        capture().map_err(|error| HarnessError::new(format!("{}: {error}", method.as_str())))?;
    Ok(MethodCapture {
        method,
        width: frame.width,
        height: frame.height,
        marker_pixels: pixels::count_near(&frame.pixels, color, tolerance),
    })
}

fn capture_gdi(hwnd: isize) -> Result<Frame, HarnessError> {
    super::probe::nudge_window(hwnd);
    let width = unsafe { GetSystemMetrics(SM_CXSCREEN) };
    let height = unsafe { GetSystemMetrics(SM_CYSCREEN) };
    let primary = rect_capture(0, 0, width, height);
    let virtual_w = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    let virtual_h = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
    let virtual_shot = if virtual_w > 0 && virtual_h > 0 {
        rect_capture(
            unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) },
            unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) },
            virtual_w,
            virtual_h,
        )
    } else {
        Err(HarnessError::new("virtual screen is empty"))
    };
    match (primary, virtual_shot) {
        (Ok(left), Ok(right)) => {
            let left_count = pixels::count_near(&left.pixels, MARKER, 12);
            let right_count = pixels::count_near(&right.pixels, MARKER, 12);
            if right_count > left_count {
                Ok(right)
            } else {
                Ok(left)
            }
        }
        (Ok(frame), Err(_)) | (Err(_), Ok(frame)) => Ok(frame),
        (Err(left), Err(right)) => Err(HarnessError::new(format!("{left}; {right}"))),
    }
}

fn rect_capture(x: i32, y: i32, width: i32, height: i32) -> Result<Frame, HarnessError> {
    rect_from_dc(None, x, y, width, height)
}

fn rect_from_dc(
    window: Option<HWND>,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> Result<Frame, HarnessError> {
    if width <= 0 || height <= 0 {
        return Err(HarnessError::new(format!(
            "capture size is {width}x{height}"
        )));
    }
    unsafe {
        let screen = GetDC(window);
        if screen.0.is_null() {
            return Err(HarnessError::new("screen dc is empty"));
        }
        let memory = CreateCompatibleDC(Some(screen));
        if memory.0.is_null() {
            let _ = ReleaseDC(window, screen);
            return Err(HarnessError::new("memory dc failed"));
        }
        let bitmap = CreateCompatibleBitmap(screen, width, height);
        if bitmap.0.is_null() {
            let _ = DeleteDC(memory);
            let _ = ReleaseDC(window, screen);
            return Err(HarnessError::new("bitmap failed"));
        }
        let previous = SelectObject(memory, HGDIOBJ(bitmap.0));
        let copied = BitBlt(memory, 0, 0, width, height, Some(screen), x, y, SRCCOPY);
        let image = match copied {
            Ok(()) => read_bitmap(memory, bitmap, width, height),
            Err(error) => Err(explain(error, "BitBlt")),
        };
        let _ = SelectObject(memory, previous);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(memory);
        let _ = ReleaseDC(window, screen);
        image
    }
}

unsafe fn read_bitmap(
    dc: windows::Win32::Graphics::Gdi::HDC,
    bitmap: windows::Win32::Graphics::Gdi::HBITMAP,
    width: i32,
    height: i32,
) -> Result<Frame, HarnessError> {
    unsafe {
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: height,
                biPlanes: 1,
                biBitCount: 32,
                ..BITMAPINFOHEADER::default()
            },
            ..BITMAPINFO::default()
        };
        let mut pixels = vec![0u8; (width as usize) * (height as usize) * 4];
        let rows = GetDIBits(
            dc,
            bitmap,
            0,
            height as u32,
            Some(pixels.as_mut_ptr().cast()),
            &mut info,
            DIB_RGB_COLORS,
        );
        if rows == 0 {
            return Err(HarnessError::new("GetDIBits returned no rows"));
        }
        flip_rows(&mut pixels, width as usize, height as usize);
        Ok(Frame {
            width: width as u32,
            height: height as u32,
            pixels,
        })
    }
}

fn flip_rows(pixels: &mut [u8], width: usize, height: usize) {
    let row = width * 4;
    let mut scratch = vec![0u8; row];
    for y in 0..height / 2 {
        let top = y * row;
        let bottom = (height - 1 - y) * row;
        scratch.copy_from_slice(&pixels[top..top + row]);
        pixels.copy_within(bottom..bottom + row, top);
        pixels[bottom..bottom + row].copy_from_slice(&scratch);
    }
}

fn capture_dxgi(hwnd: isize) -> Result<Frame, HarnessError> {
    match duplicate_from_hardware(hwnd) {
        Ok(frame) => Ok(frame),
        Err(first) => match duplicate_from_factory(hwnd) {
            Ok(frame) => Ok(frame),
            Err(second) => Err(HarnessError::new(format!("{first}; {second}"))),
        },
    }
}

fn duplicate_from_hardware(hwnd: isize) -> Result<Frame, HarnessError> {
    let (device, context) = hardware_device()?;
    let dxgi: IDXGIDevice = device
        .cast()
        .map_err(|error| explain(error, "IDXGIDevice"))?;
    let adapter = unsafe {
        dxgi.GetAdapter()
            .map_err(|error| explain(error, "GetAdapter"))?
    };
    duplicate_adapter_outputs(&adapter, &device, &context, hwnd)
}

fn duplicate_from_factory(hwnd: isize) -> Result<Frame, HarnessError> {
    let factory: IDXGIFactory1 =
        unsafe { CreateDXGIFactory1().map_err(|error| explain(error, "CreateDXGIFactory1"))? };
    let mut errors = Vec::new();
    let mut index = 0u32;
    loop {
        let adapter1 = match unsafe { factory.EnumAdapters1(index) } {
            Ok(adapter) => adapter,
            Err(_) => break,
        };
        index += 1;
        let adapter: IDXGIAdapter = match adapter1.cast() {
            Ok(adapter) => adapter,
            Err(error) => {
                errors.push(explain(error, "adapter cast").to_string());
                continue;
            }
        };
        let (device, context) = match device_on_adapter(&adapter) {
            Ok(device) => device,
            Err(error) => {
                errors.push(error.to_string());
                continue;
            }
        };
        match duplicate_adapter_outputs(&adapter, &device, &context, hwnd) {
            Ok(frame) => return Ok(frame),
            Err(error) => errors.push(error.to_string()),
        }
    }
    Err(HarnessError::new(if errors.is_empty() {
        "no adapter produced a desktop frame".to_string()
    } else {
        errors.join("; ")
    }))
}

fn duplicate_adapter_outputs(
    adapter: &IDXGIAdapter,
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    hwnd: isize,
) -> Result<Frame, HarnessError> {
    let mut errors = Vec::new();
    for output_index in 0..8 {
        let output = match unsafe { adapter.EnumOutputs(output_index) } {
            Ok(output) => output,
            Err(_) => break,
        };
        let output1: IDXGIOutput1 = match output.cast() {
            Ok(output) => output,
            Err(error) => {
                errors.push(explain(error, "IDXGIOutput1").to_string());
                continue;
            }
        };
        let duplication = match unsafe { output1.DuplicateOutput(device) } {
            Ok(duplication) => duplication,
            Err(error) => {
                errors.push(explain(error, "DuplicateOutput").to_string());
                continue;
            }
        };
        match acquire_frame(&duplication, device, context, hwnd) {
            Ok(frame) => return Ok(frame),
            Err(error) => errors.push(error.to_string()),
        }
    }
    Err(HarnessError::new(if errors.is_empty() {
        "adapter has no outputs".to_string()
    } else {
        errors.join("; ")
    }))
}

fn acquire_frame(
    duplication: &IDXGIOutputDuplication,
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    hwnd: isize,
) -> Result<Frame, HarnessError> {
    let mut best: Option<Frame> = None;
    for _ in 0..12 {
        super::probe::nudge_window(hwnd);
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource: Option<IDXGIResource> = None;
        let acquired = unsafe { duplication.AcquireNextFrame(250, &mut info, &mut resource) };
        match acquired {
            Ok(()) => {
                let resource = resource
                    .ok_or_else(|| HarnessError::new("duplication returned no resource"))?;
                let texture: ID3D11Texture2D = resource
                    .cast()
                    .map_err(|error| explain(error, "duplication texture"))?;
                let image = copy_texture(device, context, &texture);
                unsafe {
                    let _ = duplication.ReleaseFrame();
                }
                let frame = image?;
                if marker_count(&frame) >= ENOUGH_MARKER_PIXELS {
                    return Ok(frame);
                }
                best = prefer_marker(best, frame);
            }
            Err(error) if error.code() == DXGI_ERROR_WAIT_TIMEOUT => continue,
            Err(error) if error.code() == DXGI_ERROR_ACCESS_LOST => {
                return Err(explain(error, "desktop duplication lost access"));
            }
            Err(error) => return Err(explain(error, "AcquireNextFrame")),
        }
    }
    best.ok_or_else(|| HarnessError::new("desktop duplication timed out"))
}

fn capture_wgc(window: HWND, hwnd: isize) -> Result<Frame, HarnessError> {
    ensure_mta()?;
    let supported = GraphicsCaptureSession::IsSupported()
        .map_err(|error| explain(error, "GraphicsCaptureSession::IsSupported"))?;
    if !supported {
        return Err(HarnessError::new(
            "Windows.Graphics.Capture is not supported",
        ));
    }
    let (device, context) = hardware_device()?;
    let dxgi: IDXGIDevice = device
        .cast()
        .map_err(|error| explain(error, "IDXGIDevice"))?;
    let inspectable = unsafe {
        CreateDirect3D11DeviceFromDXGIDevice(&dxgi)
            .map_err(|error| explain(error, "CreateDirect3D11DeviceFromDXGIDevice"))?
    };
    let direct3d: IDirect3DDevice = inspectable
        .cast()
        .map_err(|error| explain(error, "IDirect3DDevice"))?;
    let monitor = unsafe {
        windows::Win32::Graphics::Gdi::MonitorFromWindow(
            window,
            windows::Win32::Graphics::Gdi::MONITOR_DEFAULTTOPRIMARY,
        )
    };
    if monitor.0.is_null() {
        return Err(HarnessError::new("no monitor for the probe window"));
    }
    let interop: IGraphicsCaptureItemInterop =
        windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
            .map_err(|error| explain(error, "IGraphicsCaptureItemInterop"))?;
    let item: GraphicsCaptureItem = unsafe {
        interop
            .CreateForMonitor(monitor)
            .map_err(|error| explain(error, "CreateForMonitor"))?
    };
    let size = item
        .Size()
        .map_err(|error| explain(error, "capture item size"))?;
    let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
        &direct3d,
        DirectXPixelFormat::B8G8R8A8UIntNormalized,
        2,
        size,
    )
    .map_err(|error| explain(error, "CreateFreeThreaded"))?;
    let session = pool
        .CreateCaptureSession(&item)
        .map_err(|error| explain(error, "CreateCaptureSession"))?;
    let _ = session.SetIsCursorCaptureEnabled(false);
    session
        .StartCapture()
        .map_err(|error| explain(error, "StartCapture"))?;
    let mut best: Option<Frame> = None;
    for _ in 0..20 {
        super::probe::nudge_window(hwnd);
        if let Ok(frame) = pool.TryGetNextFrame() {
            let surface = frame
                .Surface()
                .map_err(|error| explain(error, "frame surface"))?;
            let access: IDirect3DDxgiInterfaceAccess = surface
                .cast()
                .map_err(|error| explain(error, "surface access"))?;
            let texture: ID3D11Texture2D = unsafe {
                access
                    .GetInterface()
                    .map_err(|error| explain(error, "GetInterface"))?
            };
            let image = copy_texture(&device, &context, &texture)?;
            if marker_count(&image) >= ENOUGH_MARKER_PIXELS {
                let _ = session.Close();
                let _ = pool.Close();
                return Ok(image);
            }
            best = prefer_marker(best, image);
        }
    }
    let _ = session.Close();
    let _ = pool.Close();
    best.ok_or_else(|| HarnessError::new("Windows.Graphics.Capture produced no frame"))
}

const ENOUGH_MARKER_PIXELS: u64 = 8_000;

fn marker_count(frame: &Frame) -> u64 {
    pixels::count_near(&frame.pixels, MARKER, 12)
}

fn prefer_marker(current: Option<Frame>, next: Frame) -> Option<Frame> {
    match current {
        Some(current) if marker_count(&current) >= marker_count(&next) => Some(current),
        _ => Some(next),
    }
}

fn copy_texture(
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    texture: &ID3D11Texture2D,
) -> Result<Frame, HarnessError> {
    unsafe {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        texture.GetDesc(&mut desc);
        desc.Usage = D3D11_USAGE_STAGING;
        desc.BindFlags = 0;
        desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
        desc.MiscFlags = 0;
        let mut staging = None;
        device
            .CreateTexture2D(&desc, None, Some(&mut staging))
            .map_err(|error| explain(error, "staging texture"))?;
        let staging = staging.ok_or_else(|| HarnessError::new("staging texture was empty"))?;
        let source: ID3D11Resource = texture
            .cast()
            .map_err(|error| explain(error, "source resource"))?;
        let destination: ID3D11Resource = staging
            .cast()
            .map_err(|error| explain(error, "staging resource"))?;
        context.CopyResource(&destination, &source);
        context.Flush();
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        context
            .Map(&destination, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
            .map_err(|error| explain(error, "Map"))?;
        let image = read_mapped(&desc, &mapped);
        context.Unmap(&destination, 0);
        image
    }
}

fn read_mapped(
    desc: &D3D11_TEXTURE2D_DESC,
    mapped: &D3D11_MAPPED_SUBRESOURCE,
) -> Result<Frame, HarnessError> {
    if mapped.pData.is_null() || desc.Width == 0 || desc.Height == 0 {
        return Err(HarnessError::new("mapped desktop texture is empty"));
    }
    let width = desc.Width as usize;
    let height = desc.Height as usize;
    let pitch = mapped.RowPitch as usize;
    if pitch < width * 4 {
        return Err(HarnessError::new(format!(
            "row pitch {pitch} is shorter than {width} pixels"
        )));
    }
    let mut pixels = Vec::with_capacity(width * height * 4);
    unsafe {
        let src = mapped.pData as *const u8;
        for y in 0..height {
            let row = std::slice::from_raw_parts(src.add(y * pitch), width * 4);
            pixels.extend_from_slice(row);
        }
    }
    Ok(Frame {
        width: desc.Width,
        height: desc.Height,
        pixels,
    })
}

fn hardware_device() -> Result<(ID3D11Device, ID3D11DeviceContext), HarnessError> {
    create_device(None::<&IDXGIAdapter>, D3D_DRIVER_TYPE_HARDWARE)
}

fn device_on_adapter(
    adapter: &IDXGIAdapter,
) -> Result<(ID3D11Device, ID3D11DeviceContext), HarnessError> {
    create_device(adapter, D3D_DRIVER_TYPE_UNKNOWN)
}

fn create_device<P>(
    adapter: P,
    driver: windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE,
) -> Result<(ID3D11Device, ID3D11DeviceContext), HarnessError>
where
    P: windows::core::Param<IDXGIAdapter>,
{
    let mut device = None;
    let mut context = None;
    unsafe {
        D3D11CreateDevice(
            adapter,
            driver,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
        .map_err(|error| explain(error, "D3D11CreateDevice"))?;
    }
    Ok((
        device.ok_or_else(|| HarnessError::new("D3D11 device was empty"))?,
        context.ok_or_else(|| HarnessError::new("D3D11 context was empty"))?,
    ))
}

fn ensure_mta() -> Result<(), HarnessError> {
    let result = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
    match result {
        Ok(()) => Ok(()),
        Err(error) => {
            let code = error.code().0 as u32;
            if code == 0x8001_0106 || code == 1 {
                Ok(())
            } else {
                Err(explain(error, "RoInitialize"))
            }
        }
    }
}
