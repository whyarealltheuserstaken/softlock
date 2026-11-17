//! A short exclusive-fullscreen window. Esc leaves it. So does the timeout.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device,
    ID3D11RenderTargetView, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, DXGI_CREATE_FACTORY_FLAGS, DXGI_PRESENT, DXGI_SCALING_STRETCH,
    DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG_ALLOW_MODE_SWITCH, DXGI_SWAP_EFFECT_DISCARD,
    DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIFactory2, IDXGIOutput, IDXGISwapChain,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
use windows::Win32::UI::WindowsAndMessaging::{
    CS_OWNDC, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, MSG, PM_REMOVE,
    PeekMessageW, PostQuitMessage, RegisterClassW, SW_SHOW, ShowWindow, TranslateMessage,
    WINDOW_EX_STYLE, WM_CLOSE, WM_DESTROY, WM_KEYDOWN, WM_QUIT, WNDCLASSW, WS_OVERLAPPEDWINDOW,
};
use windows::core::{Interface, PCWSTR};

use crate::win::wide;

static STOP: AtomicBool = AtomicBool::new(false);

pub fn run(seconds: u64) -> Result<(), String> {
    STOP.store(false, Ordering::SeqCst);
    let seconds = seconds.clamp(1, crate::FULLSCREEN_MAX_SECS);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(seconds));
        STOP.store(true, Ordering::SeqCst);
    });
    let class_name = wide("spike-overlay-fullscreen");
    let module = unsafe { GetModuleHandleW(PCWSTR::null()) }.map_err(|error| error.to_string())?;
    let instance = HINSTANCE::from(module);
    unsafe {
        let class = WNDCLASSW {
            style: CS_OWNDC,
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        let _ = RegisterClassW(&class);
    }
    let title = wide("fullscreen spike");
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            PCWSTR(class_name.as_ptr()),
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPEDWINDOW,
            100,
            100,
            640,
            480,
            None,
            None,
            Some(instance),
            None,
        )
    }
    .map_err(|error| error.to_string())?;
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
    }
    let swap = match create_swapchain(hwnd) {
        Ok(swap) => swap,
        Err(error) => {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            return Err(error);
        }
    };
    let entered = unsafe { swap.SetFullscreenState(true, None::<&IDXGIOutput>) };
    let mut fullscreen = windows::core::BOOL(0);
    if let Err(error) = &entered {
        eprintln!("exclusive fullscreen did not start ({error}). The green window stays windowed.");
    }
    if entered.is_ok()
        && unsafe { swap.GetFullscreenState(Some(&mut fullscreen), None).is_ok() }
        && fullscreen.as_bool()
    {
        println!("fullscreen_mode: exclusive");
    } else {
        println!("fullscreen_mode: windowed");
    }
    println!("Green full-screen window. Press Esc. It also closes itself.");
    let view = render_target(&swap)?;
    let started = Instant::now();
    let exit = loop {
        pump_once();
        if STOP.load(Ordering::SeqCst) || started.elapsed() >= Duration::from_secs(seconds) {
            break "timeout";
        }
        draw(&swap, &view);
        std::thread::sleep(Duration::from_millis(16));
    };
    unsafe {
        let _ = swap.SetFullscreenState(false, None::<&IDXGIOutput>);
        let _ = DestroyWindow(hwnd);
    }
    println!("fullscreen left by {exit}");
    Ok(())
}

fn create_swapchain(hwnd: HWND) -> Result<IDXGISwapChain, String> {
    let device = create_device()?;
    let factory: IDXGIFactory2 = unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)) }
        .map_err(|error| error.to_string())?;
    let desc = DXGI_SWAP_CHAIN_DESC1 {
        Width: 0,
        Height: 0,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        Stereo: windows::core::BOOL(0),
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        BufferCount: 1,
        Scaling: DXGI_SCALING_STRETCH,
        SwapEffect: DXGI_SWAP_EFFECT_DISCARD,
        AlphaMode: DXGI_ALPHA_MODE_IGNORE,
        Flags: DXGI_SWAP_CHAIN_FLAG_ALLOW_MODE_SWITCH.0 as u32,
    };
    let swap1 =
        unsafe { factory.CreateSwapChainForHwnd(&device, hwnd, &desc, None, None::<&IDXGIOutput>) }
            .map_err(|error| error.to_string())?;
    swap1.cast().map_err(|error| error.to_string())
}

fn create_device() -> Result<ID3D11Device, String> {
    let levels = [D3D_FEATURE_LEVEL_11_0];
    for driver in [D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP] {
        let mut device = None;
        let mut context = None;
        let created = unsafe {
            D3D11CreateDevice(
                None::<&windows::Win32::Graphics::Dxgi::IDXGIAdapter>,
                driver,
                Default::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&levels),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
        };
        if created.is_ok()
            && let Some(device) = device
        {
            return Ok(device);
        }
    }
    Err("Direct3D did not start".into())
}

fn render_target(swap: &IDXGISwapChain) -> Result<ID3D11RenderTargetView, String> {
    let texture: ID3D11Texture2D =
        unsafe { swap.GetBuffer(0) }.map_err(|error| error.to_string())?;
    let device: ID3D11Device = unsafe { swap.GetDevice() }.map_err(|error| error.to_string())?;
    let mut view = None;
    unsafe {
        device
            .CreateRenderTargetView(&texture, None, Some(&mut view))
            .map_err(|error| error.to_string())?;
    }
    view.ok_or_else(|| "no render target".to_string())
}

fn draw(swap: &IDXGISwapChain, view: &ID3D11RenderTargetView) {
    let Ok(device) = (unsafe { swap.GetDevice::<ID3D11Device>() }) else {
        return;
    };
    let Ok(context) = (unsafe { device.GetImmediateContext() }) else {
        return;
    };
    let green = [0.0_f32, 0.75, 0.05, 1.0];
    unsafe {
        context.ClearRenderTargetView(view, &green);
        let _ = swap.Present(1, DXGI_PRESENT(0));
    }
}

fn pump_once() {
    unsafe {
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            if msg.message == WM_QUIT {
                STOP.store(true, Ordering::SeqCst);
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_KEYDOWN && wparam.0 == VK_ESCAPE.0 as usize {
            STOP.store(true, Ordering::SeqCst);
            PostQuitMessage(0);
            return LRESULT(0);
        }
        if msg == WM_CLOSE {
            STOP.store(true, Ordering::SeqCst);
            PostQuitMessage(0);
            return LRESULT(0);
        }
        if msg == WM_DESTROY {
            return LRESULT(0);
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}
