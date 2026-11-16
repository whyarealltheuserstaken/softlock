use std::path::Path;

use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::HMODULE;
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
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromWindow,
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

use crate::bmp::write_bmp;
use crate::report::ShotReport;
use crate::verdict::{self, BgraImage, ScreenRect};

use super::util::explain;
use super::window::{self, MarkerWindows};

struct Frame {
    image: BgraImage,
    origin_x: i32,
    origin_y: i32,
}

pub fn capture_all(windows: &MarkerWindows, output: &Path, exclusion: bool) -> Vec<ShotReport> {
    let _ = ensure_mta();
    let marker = windows.marker_rect;
    let adjacent = windows.adjacent_rect;
    vec![
        one("gdi", exclusion, output, marker, adjacent, || {
            capture_gdi(marker, adjacent)
        }),
        one("dxgi", exclusion, output, marker, adjacent, || {
            capture_dxgi(windows)
        }),
        one("wgc", exclusion, output, marker, adjacent, || {
            capture_wgc(windows)
        }),
    ]
}

fn one(
    method: &str,
    exclusion: bool,
    output: &Path,
    marker: ScreenRect,
    adjacent: ScreenRect,
    capture: impl FnOnce() -> Result<Frame, String>,
) -> ShotReport {
    let phase = if exclusion { "on" } else { "off" };
    match capture() {
        Ok(frame) => {
            let counts = verdict::counts(&frame.image);
            let verdict = verdict::judge(counts);
            let bmp_name = format!("{method}-exclusion-{phase}.bmp");
            let bmp_path = output.join(&bmp_name);
            let mut note = write_bmp(&bmp_path, &frame.image)
                .err()
                .map(|error| error.to_string());
            let marker_region = verdict::count_rgb_rect(
                &frame.image,
                frame.origin_x,
                frame.origin_y,
                marker,
                verdict::MARKER,
                2,
            );
            let marker_region_control = verdict::count_rgb_rect(
                &frame.image,
                frame.origin_x,
                frame.origin_y,
                marker,
                verdict::CONTROL,
                2,
            );
            let adjacent_control = verdict::count_rgb_rect(
                &frame.image,
                frame.origin_x,
                frame.origin_y,
                adjacent,
                verdict::CONTROL,
                2,
            );
            if marker_region.is_none() || adjacent_control.is_none() {
                let outside = format!(
                    "sample region is outside the capture origin={},{} size={}x{}",
                    frame.origin_x, frame.origin_y, frame.image.width, frame.image.height
                );
                note = Some(match note {
                    Some(existing) => format!("{existing}; {outside}"),
                    None => outside,
                });
            }
            ShotReport {
                method: method.to_string(),
                exclusion,
                marker_pixels: counts.marker,
                control_pixels: counts.control,
                marker_region_pixels: marker_region.unwrap_or(0),
                marker_region_control_pixels: marker_region_control.unwrap_or(0),
                adjacent_control_pixels: adjacent_control.unwrap_or(0),
                verdict: verdict.as_str().to_string(),
                note,
                error: None,
                bmp: Some(bmp_name),
            }
        }
        Err(error) => ShotReport {
            method: method.to_string(),
            exclusion,
            marker_pixels: 0,
            control_pixels: 0,
            marker_region_pixels: 0,
            marker_region_control_pixels: 0,
            adjacent_control_pixels: 0,
            verdict: verdict::Verdict::Unusable.as_str().to_string(),
            note: None,
            error: Some(error),
            bmp: None,
        },
    }
}

fn capture_gdi(marker: ScreenRect, adjacent: ScreenRect) -> Result<Frame, String> {
    let primary = rect_capture(
        0,
        0,
        unsafe { GetSystemMetrics(SM_CXSCREEN) },
        unsafe { GetSystemMetrics(SM_CYSCREEN) },
        "primary",
    );
    let virtual_w = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    let virtual_h = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
    let virtual_shot = if virtual_w > 0
        && virtual_h > 0
        && i64::from(virtual_w) * i64::from(virtual_h) <= 8_000_000
    {
        rect_capture(
            unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) },
            unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) },
            virtual_w,
            virtual_h,
            "virtual",
        )
    } else {
        Err("virtual screen skipped".to_string())
    };
    choose_image(primary, virtual_shot, marker, adjacent)
}

fn rect_capture(x: i32, y: i32, width: i32, height: i32, label: &str) -> Result<Frame, String> {
    if width <= 0 || height <= 0 {
        return Err(format!("{label} capture size is {width}x{height}"));
    }
    unsafe {
        let screen = GetDC(None);
        if screen.0.is_null() {
            return Err(format!("{label} screen dc is empty"));
        }
        let memory = CreateCompatibleDC(Some(screen));
        if memory.0.is_null() {
            let _ = ReleaseDC(None, screen);
            return Err(format!("{label} memory dc failed"));
        }
        let bitmap = CreateCompatibleBitmap(screen, width, height);
        if bitmap.0.is_null() {
            let _ = DeleteDC(memory);
            let _ = ReleaseDC(None, screen);
            return Err(format!("{label} bitmap failed"));
        }
        let previous = SelectObject(memory, HGDIOBJ(bitmap.0));
        let copied = BitBlt(memory, 0, 0, width, height, Some(screen), x, y, SRCCOPY);
        let _ = SelectObject(memory, previous);
        let image = match copied {
            Ok(()) => read_bitmap(memory, bitmap, width, height, label).map(|image| Frame {
                image,
                origin_x: x,
                origin_y: y,
            }),
            Err(error) => Err(explain(error, &format!("{label} BitBlt"))),
        };
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(memory);
        let _ = ReleaseDC(None, screen);
        image
    }
}

unsafe fn read_bitmap(
    dc: windows::Win32::Graphics::Gdi::HDC,
    bitmap: windows::Win32::Graphics::Gdi::HBITMAP,
    width: i32,
    height: i32,
    label: &str,
) -> Result<BgraImage, String> {
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
            return Err(format!("{label} GetDIBits returned no rows"));
        }
        flip_rows(&mut pixels, width as usize, height as usize);
        BgraImage::new(width as usize, height as usize, pixels)
            .ok_or_else(|| format!("{label} image was empty"))
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

fn choose_image(
    first: Result<Frame, String>,
    second: Result<Frame, String>,
    marker: ScreenRect,
    adjacent: ScreenRect,
) -> Result<Frame, String> {
    match (first, second) {
        (Ok(left), Ok(right)) => {
            let left_covers = frame_covers(&left, marker, adjacent);
            let right_covers = frame_covers(&right, marker, adjacent);
            if right_covers && !left_covers {
                return Ok(right);
            }
            if left_covers && !right_covers {
                return Ok(left);
            }
            let left_control = verdict::counts(&left.image).control;
            let right_control = verdict::counts(&right.image).control;
            if right_control > left_control {
                Ok(right)
            } else {
                Ok(left)
            }
        }
        (Ok(image), Err(_)) | (Err(_), Ok(image)) => Ok(image),
        (Err(left), Err(right)) => Err(format!("{left}; {right}")),
    }
}

fn frame_covers(frame: &Frame, marker: ScreenRect, adjacent: ScreenRect) -> bool {
    verdict::rect_inside(&frame.image, frame.origin_x, frame.origin_y, marker)
        && verdict::rect_inside(&frame.image, frame.origin_x, frame.origin_y, adjacent)
}

fn capture_dxgi(windows: &MarkerWindows) -> Result<Frame, String> {
    let mut errors = Vec::new();
    match duplicate_from_hardware(windows) {
        Ok(image) => return Ok(image),
        Err(error) => errors.push(error),
    }
    match duplicate_from_factory(windows, &mut errors) {
        Ok(image) => Ok(image),
        Err(error) => {
            errors.push(error);
            Err(errors.join("; "))
        }
    }
}

fn duplicate_from_hardware(windows: &MarkerWindows) -> Result<Frame, String> {
    let (device, context) = hardware_device()?;
    let dxgi: IDXGIDevice = device
        .cast()
        .map_err(|error| explain(error, "IDXGIDevice"))?;
    let adapter = unsafe {
        dxgi.GetAdapter()
            .map_err(|error| explain(error, "GetAdapter"))?
    };
    duplicate_adapter_outputs(&adapter, &device, &context, windows)
}

fn duplicate_from_factory(
    windows: &MarkerWindows,
    errors: &mut Vec<String>,
) -> Result<Frame, String> {
    let factory: IDXGIFactory1 =
        unsafe { CreateDXGIFactory1().map_err(|error| explain(error, "CreateDXGIFactory1"))? };
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
                errors.push(explain(error, "adapter cast"));
                continue;
            }
        };
        let device = match device_on_adapter(&adapter) {
            Ok(device) => device,
            Err(error) => {
                errors.push(error);
                continue;
            }
        };
        match duplicate_adapter_outputs(&adapter, &device.0, &device.1, windows) {
            Ok(image) => return Ok(image),
            Err(error) => errors.push(error),
        }
        let _ = adapter1;
    }
    Err("no adapter produced a desktop frame".to_string())
}

fn duplicate_adapter_outputs(
    adapter: &IDXGIAdapter,
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    windows: &MarkerWindows,
) -> Result<Frame, String> {
    let mut errors = Vec::new();
    for output_index in 0..8 {
        let output = match unsafe { adapter.EnumOutputs(output_index) } {
            Ok(output) => output,
            Err(_) => break,
        };
        let output1: IDXGIOutput1 = match output.cast() {
            Ok(output) => output,
            Err(error) => {
                errors.push(explain(error, "IDXGIOutput1"));
                continue;
            }
        };
        let origin = match unsafe { output.GetDesc() } {
            Ok(desc) => (desc.DesktopCoordinates.left, desc.DesktopCoordinates.top),
            Err(error) => {
                errors.push(explain(error, "output description"));
                (0, 0)
            }
        };
        let duplication = match unsafe { output1.DuplicateOutput(device) } {
            Ok(duplication) => duplication,
            Err(error) => {
                errors.push(explain(error, &format!("DuplicateOutput {output_index}")));
                continue;
            }
        };
        match acquire_frame(&duplication, device, context, windows, origin) {
            Ok(image) => return Ok(image),
            Err(error) => errors.push(error),
        }
    }
    if errors.is_empty() {
        Err("adapter has no outputs".to_string())
    } else {
        Err(errors.join("; "))
    }
}

fn acquire_frame(
    duplication: &IDXGIOutputDuplication,
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    windows: &MarkerWindows,
    origin: (i32, i32),
) -> Result<Frame, String> {
    let mut lost = false;
    for _ in 0..12 {
        window::nudge(windows);
        window::pump(30);
        let mut info: DXGI_OUTDUPL_FRAME_INFO = unsafe { std::mem::zeroed() };
        let mut resource: Option<IDXGIResource> = None;
        let acquired = unsafe { duplication.AcquireNextFrame(250, &mut info, &mut resource) };
        match acquired {
            Ok(()) => {
                let resource = resource.ok_or("desktop duplication returned no resource")?;
                let texture: ID3D11Texture2D = resource
                    .cast()
                    .map_err(|error| explain(error, "duplication texture"))?;
                let image = copy_texture(device, context, &texture).map(|image| Frame {
                    image,
                    origin_x: origin.0,
                    origin_y: origin.1,
                });
                unsafe {
                    let _ = duplication.ReleaseFrame();
                }
                return image;
            }
            Err(error) if error.code() == DXGI_ERROR_WAIT_TIMEOUT => continue,
            Err(error) if error.code() == DXGI_ERROR_ACCESS_LOST => {
                lost = true;
                break;
            }
            Err(error) => return Err(explain(error, "AcquireNextFrame")),
        }
    }
    if lost {
        Err("desktop duplication lost access".to_string())
    } else {
        Err("desktop duplication timed out".to_string())
    }
}

fn capture_wgc(windows: &MarkerWindows) -> Result<Frame, String> {
    ensure_mta()?;
    let supported = GraphicsCaptureSession::IsSupported()
        .map_err(|error| explain(error, "GraphicsCaptureSession::IsSupported"))?;
    if !supported {
        return Err("Windows.Graphics.Capture is not supported".to_string());
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
    let monitor = unsafe { MonitorFromWindow(windows.marker, MONITOR_DEFAULTTOPRIMARY) };
    if monitor.0.is_null() {
        return Err("no monitor for the marker window".to_string());
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
    let mut frame_texture = None;
    for _ in 0..20 {
        window::nudge(windows);
        window::pump(40);
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
            frame_texture = Some(texture);
            break;
        }
    }
    let texture = frame_texture.ok_or("Windows.Graphics.Capture produced no frame")?;
    let image = copy_texture(&device, &context, &texture);
    let _ = session.Close();
    let _ = pool.Close();
    let (origin_x, origin_y) = monitor_origin(monitor);
    image.map(|image| Frame {
        image,
        origin_x,
        origin_y,
    })
}

fn monitor_origin(monitor: HMONITOR) -> (i32, i32) {
    let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    let ok = unsafe { GetMonitorInfoW(monitor, &mut info) };
    if ok.as_bool() {
        (info.rcMonitor.left, info.rcMonitor.top)
    } else {
        (0, 0)
    }
}

fn copy_texture(
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    texture: &ID3D11Texture2D,
) -> Result<BgraImage, String> {
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
        let staging = staging.ok_or("staging texture was empty")?;
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
) -> Result<BgraImage, String> {
    if mapped.pData.is_null() || desc.Width == 0 || desc.Height == 0 {
        return Err("mapped desktop texture is empty".to_string());
    }
    let width = desc.Width as usize;
    let height = desc.Height as usize;
    let pitch = mapped.RowPitch as usize;
    if pitch < width * 4 {
        return Err(format!("row pitch {pitch} is shorter than {width} pixels"));
    }
    let mut pixels = Vec::with_capacity(width * height * 4);
    unsafe {
        let src = mapped.pData as *const u8;
        for y in 0..height {
            let row = std::slice::from_raw_parts(src.add(y * pitch), width * 4);
            pixels.extend_from_slice(row);
        }
    }
    BgraImage::new(width, height, pixels).ok_or_else(|| "desktop image was empty".to_string())
}

fn hardware_device() -> Result<(ID3D11Device, ID3D11DeviceContext), String> {
    create_device(None::<&IDXGIAdapter>, D3D_DRIVER_TYPE_HARDWARE)
}

fn device_on_adapter(
    adapter: &IDXGIAdapter,
) -> Result<(ID3D11Device, ID3D11DeviceContext), String> {
    create_device(adapter, D3D_DRIVER_TYPE_UNKNOWN)
}

fn create_device<P>(
    adapter: P,
    driver: windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE,
) -> Result<(ID3D11Device, ID3D11DeviceContext), String>
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
        device.ok_or("D3D11 device was empty")?,
        context.ok_or("D3D11 context was empty")?,
    ))
}

fn ensure_mta() -> Result<(), String> {
    let result = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
    match result {
        Ok(()) => Ok(()),
        Err(error) => {
            let code = error.code().0 as u32;
            // RPC_E_CHANGED_MODE or S_FALSE: the thread is already initialized.
            if code == 0x8001_0106 || code == 1 {
                Ok(())
            } else {
                Err(explain(error, "RoInitialize"))
            }
        }
    }
}
