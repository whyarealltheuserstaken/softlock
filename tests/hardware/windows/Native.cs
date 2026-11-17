using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

namespace SoftLock.Hardware
{
    public static class SoftLockHw
    {
        const int WhKeyboardLl = 13;
        const int WmKeyDown = 0x0100;
        const int LlkhfInjected = 0x10;
        const int VkF23 = 0x86;
        const uint EsContinuous = 0x80000000;
        const uint EsSystemRequired = 0x00000001;
        const uint WdaExcludeFromCapture = 0x00000011;
        const uint WsExTopmost = 0x00000008;
        const uint WsExTransparent = 0x00000020;
        const uint WsExToolWindow = 0x00000080;
        const uint WsExLayered = 0x00080000;
        const uint WsExNoActivate = 0x08000000;
        const uint WsPopup = 0x80000000;
        const uint Marker = 0x00A00ADC;
        const uint WmNcHitTest = 0x0084;
        const uint WmPowerBroadcast = 0x0218;
        const uint WmWtsSessionChange = 0x02B1;
        const int WtsSessionLock = 0x7;
        const int WtsSessionUnlock = 0x8;
        const int PbtApmSuspend = 0x0004;
        const int PbtApmResumeSuspend = 0x0007;
        const int PbtApmResumeAutomatic = 0x0012;
        const int PbtPowerSettingChange = 0x8013;
        static readonly Guid LidGuid = new Guid("BA3E0F4D-B817-4094-A2D1-D56379E6A0F3");
        static readonly Guid DisplayGuid = new Guid("6FE69556-704A-47A0-8F24-C28D936FDA47");

        static readonly object Gate = new object();
        static HookProc hookProc;
        static IntPtr hook = IntPtr.Zero;
        static int injectedSeen;
        static WndProc overlayProc;
        static WndProc listenerProc;
        static IntPtr listenerHwnd = IntPtr.Zero;
        static Thread listenerThread;
        static uint listenerThreadId;
        static readonly ManualResetEvent ListenerReady = new ManualResetEvent(false);
        static int suspends;
        static int resumes;
        static int lidCloses;
        static int lidOpens;
        static int displayOff;
        static int displayOn;
        static int sessionLocks;
        static int sessionUnlocks;
        static IntPtr powerRequest = IntPtr.Zero;
        static IntPtr reasonText = IntPtr.Zero;
        static bool dpiReady;

        public static string DetectJson()
        {
            try
            {
                EnsureDpi();
                int monitors = MonitorRects().Count;
                int protocol = ClientProtocol();
                byte ac = 255;
                SystemPowerStatus power;
                if (GetSystemPowerStatus(out power))
                {
                    ac = power.ACLineStatus;
                }
                bool admin = IsUserAnAdmin();
                return "{\"ok\":true,\"monitors\":" + monitors
                    + ",\"protocol\":" + protocol
                    + ",\"ac\":" + ac
                    + ",\"elevated\":" + (admin ? "true" : "false") + "}";
            }
            catch (Exception error)
            {
                return "{\"ok\":false,\"monitors\":0,\"protocol\":-1,\"ac\":255,\"elevated\":false,\"error\":\"" + Esc(error.Message) + "\"}";
            }
        }

        public static string OverlayJson()
        {
            List<IntPtr> windows = new List<IntPtr>();
            try
            {
                EnsureDpi();
                List<Rect> monitors = MonitorRects();
                if (monitors.Count == 0)
                {
                    return "{\"ok\":true,\"monitors\":0,\"windows\":0,\"topmost\":false,\"click_through\":false,\"covered\":false,\"affinity\":false,\"marker_visible\":false}";
                }
                overlayProc = OverlayWnd;
                IntPtr proc = Marshal.GetFunctionPointerForDelegate(overlayProc);
                IntPtr brush = CreateSolidBrush(Marker);
                WndClass wc = new WndClass();
                wc.lpfnWndProc = proc;
                wc.hInstance = GetModuleHandle(null);
                wc.hbrBackground = brush;
                wc.lpszClassName = "SoftLockHwOverlay";
                RegisterClassW(ref wc);
                bool topmost = true;
                bool click = true;
                bool covered = true;
                bool affinity = true;
                bool markerVisible = false;
                foreach (Rect monitor in monitors)
                {
                    int width = monitor.right - monitor.left;
                    int height = monitor.bottom - monitor.top;
                    IntPtr hwnd = CreateWindowExW(
                        WsExTopmost | WsExTransparent | WsExToolWindow | WsExLayered | WsExNoActivate,
                        "SoftLockHwOverlay",
                        "",
                        WsPopup,
                        monitor.left,
                        monitor.top,
                        width,
                        height,
                        IntPtr.Zero,
                        IntPtr.Zero,
                        wc.hInstance,
                        IntPtr.Zero);
                    if (hwnd == IntPtr.Zero)
                    {
                        topmost = false;
                        covered = false;
                        continue;
                    }
                    windows.Add(hwnd);
                    SetLayeredWindowAttributes(hwnd, 0, 255, 2);
                    ShowWindow(hwnd, 8);
                    uint style = (uint)GetExStyle(hwnd).ToInt64();
                    if ((style & WsExTopmost) == 0)
                    {
                        topmost = false;
                    }
                    if ((style & WsExTransparent) == 0)
                    {
                        click = false;
                    }
                    int packed = (monitor.left + (width / 2)) & 0xFFFF;
                    packed |= (monitor.top + (height / 2)) << 16;
                    IntPtr hit = SendMessageW(hwnd, WmNcHitTest, IntPtr.Zero, new IntPtr(packed));
                    if (hit.ToInt64() != -1)
                    {
                        click = false;
                    }
                    Rect got;
                    if (!GetWindowRect(hwnd, out got)
                        || got.left != monitor.left
                        || got.top != monitor.top
                        || got.right != monitor.right
                        || got.bottom != monitor.bottom)
                    {
                        covered = false;
                    }
                    if (!SetWindowDisplayAffinity(hwnd, WdaExcludeFromCapture))
                    {
                        affinity = false;
                    }
                    else
                    {
                        uint gotAffinity;
                        if (!GetWindowDisplayAffinity(hwnd, out gotAffinity) || gotAffinity != WdaExcludeFromCapture)
                        {
                            affinity = false;
                        }
                    }
                    IntPtr dc = GetDC(IntPtr.Zero);
                    if (dc != IntPtr.Zero)
                    {
                        uint pixel = GetPixel(dc, monitor.left + (width / 2), monitor.top + (height / 2));
                        ReleaseDC(IntPtr.Zero, dc);
                        if (pixel == Marker)
                        {
                            markerVisible = true;
                        }
                    }
                }
                return "{\"ok\":true,\"monitors\":" + monitors.Count
                    + ",\"windows\":" + windows.Count
                    + ",\"topmost\":" + Bool(topmost)
                    + ",\"click_through\":" + Bool(click)
                    + ",\"covered\":" + Bool(covered)
                    + ",\"affinity\":" + Bool(affinity)
                    + ",\"marker_visible\":" + Bool(markerVisible) + "}";
            }
            catch (Exception error)
            {
                return "{\"ok\":false,\"monitors\":0,\"windows\":0,\"topmost\":false,\"click_through\":false,\"covered\":false,\"affinity\":false,\"marker_visible\":false,\"error\":\"" + Esc(error.Message) + "\"}";
            }
            finally
            {
                foreach (IntPtr hwnd in windows)
                {
                    DestroyWindow(hwnd);
                }
            }
        }

        public static string HookJson()
        {
            try
            {
                injectedSeen = 0;
                hookProc = HookCallback;
                hook = SetWindowsHookEx(WhKeyboardLl, hookProc, GetModuleHandle(null), 0);
                if (hook == IntPtr.Zero)
                {
                    return "{\"ok\":false,\"installed\":false,\"injected_seen\":0,\"error\":\"hook\"}";
                }
                SendKey(VkF23, false);
                SendKey(VkF23, true);
                Thread.Sleep(50);
                return "{\"ok\":true,\"installed\":true,\"injected_seen\":" + injectedSeen + "}";
            }
            catch (Exception error)
            {
                return "{\"ok\":false,\"installed\":false,\"injected_seen\":0,\"error\":\"" + Esc(error.Message) + "\"}";
            }
            finally
            {
                ReleaseHook();
            }
        }

        public static string KeepAwakeJson()
        {
            try
            {
                ReleaseKeepAwake();
                SetThreadExecutionState(EsContinuous | EsSystemRequired);
                reasonText = Marshal.StringToHGlobalUni("SoftLock hardware suite");
                ReasonContext context = new ReasonContext();
                context.Version = 0;
                context.Flags = 0x1;
                context.SimpleReasonString = reasonText;
                powerRequest = PowerCreateRequest(ref context);
                if (powerRequest == IntPtr.Zero)
                {
                    return "{\"ok\":false,\"api\":false,\"error\":\"PowerCreateRequest\"}";
                }
                bool system = PowerSetRequest(powerRequest, 1);
                bool display = PowerSetRequest(powerRequest, 0);
                return "{\"ok\":" + Bool(system && display) + ",\"api\":" + Bool(system && display) + "}";
            }
            catch (Exception error)
            {
                return "{\"ok\":false,\"api\":false,\"error\":\"" + Esc(error.Message) + "\"}";
            }
        }

        public static void ReleaseKeepAwake()
        {
            if (powerRequest != IntPtr.Zero)
            {
                PowerClearRequest(powerRequest, 1);
                PowerClearRequest(powerRequest, 0);
                CloseHandle(powerRequest);
                powerRequest = IntPtr.Zero;
            }
            if (reasonText != IntPtr.Zero)
            {
                Marshal.FreeHGlobal(reasonText);
                reasonText = IntPtr.Zero;
            }
            SetThreadExecutionState(EsContinuous);
        }

        public static void StartListener()
        {
            if (listenerThread != null)
            {
                return;
            }
            ListenerReady.Reset();
            listenerThread = new Thread(ListenerMain);
            listenerThread.IsBackground = true;
            listenerThread.Start();
            ListenerReady.WaitOne(3000);
        }

        public static string SnapshotJson()
        {
            lock (Gate)
            {
                return "{\"suspend\":" + suspends
                    + ",\"resume\":" + resumes
                    + ",\"lid_close\":" + lidCloses
                    + ",\"lid_open\":" + lidOpens
                    + ",\"display_off\":" + displayOff
                    + ",\"display_on\":" + displayOn
                    + ",\"session_lock\":" + sessionLocks
                    + ",\"session_unlock\":" + sessionUnlocks + "}";
            }
        }

        public static string MeasureLockJson()
        {
            StartListener();
            int before;
            lock (Gate)
            {
                before = sessionLocks;
            }
            var watch = System.Diagnostics.Stopwatch.StartNew();
            bool api = LockWorkStation();
            bool locked = false;
            while (watch.ElapsedMilliseconds < 1000)
            {
                lock (Gate)
                {
                    if (sessionLocks > before)
                    {
                        locked = true;
                        break;
                    }
                }
                Thread.Sleep(20);
            }
            return "{\"ok\":true,\"api\":" + Bool(api)
                + ",\"locked\":" + Bool(locked)
                + ",\"elapsed_ms\":" + watch.ElapsedMilliseconds + "}";
        }

        public static void LockNow()
        {
            LockWorkStation();
        }

        public static bool WaitForUnlock(int milliseconds)
        {
            StartListener();
            int before;
            lock (Gate)
            {
                before = sessionUnlocks;
            }
            var watch = System.Diagnostics.Stopwatch.StartNew();
            while (watch.ElapsedMilliseconds < milliseconds)
            {
                lock (Gate)
                {
                    if (sessionUnlocks > before)
                    {
                        return true;
                    }
                }
                Thread.Sleep(50);
            }
            return false;
        }

        public static void ReleaseAll()
        {
            ReleaseHook();
            ReleaseKeepAwake();
            StopListener();
        }

        static void ReleaseHook()
        {
            if (hook != IntPtr.Zero)
            {
                UnhookWindowsHookEx(hook);
                hook = IntPtr.Zero;
            }
        }

        static void StopListener()
        {
            if (listenerThread == null)
            {
                return;
            }
            if (listenerThreadId != 0)
            {
                PostThreadMessage(listenerThreadId, 0x0012, IntPtr.Zero, IntPtr.Zero);
            }
            listenerThread.Join(1000);
            listenerThread = null;
            listenerHwnd = IntPtr.Zero;
        }

        static void ListenerMain()
        {
            listenerThreadId = GetCurrentThreadId();
            Msg discard;
            PeekMessage(out discard, IntPtr.Zero, 0, 0, 0);
            listenerProc = ListenerWnd;
            IntPtr proc = Marshal.GetFunctionPointerForDelegate(listenerProc);
            WndClass wc = new WndClass();
            wc.lpfnWndProc = proc;
            wc.hInstance = GetModuleHandle(null);
            wc.lpszClassName = "SoftLockHwListener";
            RegisterClassW(ref wc);
            listenerHwnd = CreateWindowExW(0, "SoftLockHwListener", "", 0, 0, 0, 0, 0, new IntPtr(-3), IntPtr.Zero, wc.hInstance, IntPtr.Zero);
            if (listenerHwnd != IntPtr.Zero)
            {
                WTSRegisterSessionNotification(listenerHwnd, 0);
                RegisterPowerSettingNotification(listenerHwnd, ref LidGuid, 0);
                RegisterPowerSettingNotification(listenerHwnd, ref DisplayGuid, 0);
            }
            ListenerReady.Set();
            Msg msg;
            while (GetMessage(out msg, IntPtr.Zero, 0, 0) > 0)
            {
                TranslateMessage(ref msg);
                DispatchMessage(ref msg);
            }
            if (listenerHwnd != IntPtr.Zero)
            {
                WTSUnRegisterSessionNotification(listenerHwnd);
                DestroyWindow(listenerHwnd);
            }
        }

        static IntPtr ListenerWnd(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam)
        {
            if (message == WmPowerBroadcast)
            {
                int kind = wParam.ToInt32();
                if (kind == PbtApmSuspend)
                {
                    lock (Gate) { suspends++; }
                }
                else if (kind == PbtApmResumeSuspend || kind == PbtApmResumeAutomatic)
                {
                    lock (Gate) { resumes++; }
                }
                else if (kind == PbtPowerSettingChange && lParam != IntPtr.Zero)
                {
                    NoteSetting(lParam);
                }
            }
            else if (message == WmWtsSessionChange)
            {
                int kind = wParam.ToInt32();
                if (kind == WtsSessionLock)
                {
                    lock (Gate) { sessionLocks++; }
                }
                else if (kind == WtsSessionUnlock)
                {
                    lock (Gate) { sessionUnlocks++; }
                }
            }
            return DefWindowProcW(hwnd, message, wParam, lParam);
        }

        static void NoteSetting(IntPtr lParam)
        {
            byte[] raw = new byte[24];
            Marshal.Copy(lParam, raw, 0, raw.Length);
            byte[] guidBytes = new byte[16];
            Buffer.BlockCopy(raw, 0, guidBytes, 0, 16);
            Guid setting = new Guid(guidBytes);
            int data = raw[20];
            lock (Gate)
            {
                if (setting == LidGuid)
                {
                    if (data == 0) { lidCloses++; } else { lidOpens++; }
                }
                else if (setting == DisplayGuid)
                {
                    if (data == 0) { displayOff++; } else { displayOn++; }
                }
            }
        }

        static IntPtr OverlayWnd(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam)
        {
            if (message == WmNcHitTest)
            {
                return new IntPtr(-1);
            }
            return DefWindowProcW(hwnd, message, wParam, lParam);
        }

        static IntPtr HookCallback(int code, IntPtr wParam, IntPtr lParam)
        {
            if (code >= 0 && wParam.ToInt32() == WmKeyDown && lParam != IntPtr.Zero)
            {
                int vk = Marshal.ReadInt32(lParam);
                int flags = Marshal.ReadInt32(lParam, 8);
                if (vk == VkF23 && (flags & LlkhfInjected) != 0)
                {
                    injectedSeen++;
                }
            }
            return CallNextHookEx(hook, code, wParam, lParam);
        }

        static void SendKey(int vk, bool up)
        {
            int size = IntPtr.Size == 8 ? 40 : 28;
            byte[] data = new byte[size];
            data[0] = 1;
            int at = IntPtr.Size == 8 ? 8 : 4;
            data[at] = (byte)(vk & 0xFF);
            if (up)
            {
                data[at + 4] = 2;
            }
            SendInput(1, data, size);
        }

        static List<Rect> MonitorRects()
        {
            List<Rect> found = new List<Rect>();
            MonitorEnum proc = delegate(IntPtr monitor, IntPtr hdc, ref Rect rect, IntPtr data)
            {
                found.Add(rect);
                return true;
            };
            EnumDisplayMonitors(IntPtr.Zero, IntPtr.Zero, proc, IntPtr.Zero);
            GC.KeepAlive(proc);
            return found;
        }

        static int ClientProtocol()
        {
            IntPtr buffer;
            int bytes;
            if (!WTSQuerySessionInformationW(IntPtr.Zero, -1, 16, out buffer, out bytes) || buffer == IntPtr.Zero)
            {
                return -1;
            }
            int protocol = Marshal.ReadInt16(buffer);
            WTSFreeMemory(buffer);
            return protocol;
        }

        static void EnsureDpi()
        {
            if (!dpiReady)
            {
                SetProcessDPIAware();
                dpiReady = true;
            }
        }

        static IntPtr GetExStyle(IntPtr hwnd)
        {
            if (IntPtr.Size == 8)
            {
                return GetWindowLongPtrW(hwnd, -20);
            }
            return GetWindowLongW(hwnd, -20);
        }

        static string Bool(bool value)
        {
            return value ? "true" : "false";
        }

        static string Esc(string text)
        {
            if (text == null)
            {
                return "";
            }
            return text.Replace("\\", " ").Replace("\"", "'").Replace("\r", " ").Replace("\n", " ");
        }

        delegate IntPtr HookProc(int code, IntPtr wParam, IntPtr lParam);
        delegate IntPtr WndProc(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam);
        delegate bool MonitorEnum(IntPtr monitor, IntPtr hdc, ref Rect rect, IntPtr data);

        [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
        struct WndClass
        {
            public uint style;
            public IntPtr lpfnWndProc;
            public int cbClsExtra;
            public int cbWndExtra;
            public IntPtr hInstance;
            public IntPtr hIcon;
            public IntPtr hCursor;
            public IntPtr hbrBackground;
            public string lpszMenuName;
            public string lpszClassName;
        }

        [StructLayout(LayoutKind.Sequential)]
        struct Rect
        {
            public int left;
            public int top;
            public int right;
            public int bottom;
        }

        [StructLayout(LayoutKind.Sequential)]
        struct Msg
        {
            public IntPtr hwnd;
            public uint message;
            public IntPtr wParam;
            public IntPtr lParam;
            public uint time;
            public int x;
            public int y;
        }

        [StructLayout(LayoutKind.Sequential)]
        struct SystemPowerStatus
        {
            public byte ACLineStatus;
            public byte BatteryFlag;
            public byte BatteryLifePercent;
            public byte SystemStatusFlag;
            public uint BatteryLifeTime;
            public uint BatteryFullLifeTime;
        }

        [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
        struct ReasonContext
        {
            public uint Version;
            public uint Flags;
            public IntPtr SimpleReasonString;
            public long Pad0;
            public long Pad1;
        }

        [DllImport("user32.dll")] static extern bool SetProcessDPIAware();
        [DllImport("user32.dll")] static extern bool EnumDisplayMonitors(IntPtr hdc, IntPtr clip, MonitorEnum proc, IntPtr data);
        [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)] static extern ushort RegisterClassW(ref WndClass wc);
        [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        static extern IntPtr CreateWindowExW(uint ex, string klass, string name, uint style, int x, int y, int w, int h, IntPtr parent, IntPtr menu, IntPtr instance, IntPtr param);
        [DllImport("user32.dll")] static extern bool DestroyWindow(IntPtr hwnd);
        [DllImport("user32.dll")] static extern bool ShowWindow(IntPtr hwnd, int cmd);
        [DllImport("user32.dll")] static extern bool SetLayeredWindowAttributes(IntPtr hwnd, uint key, byte alpha, uint flags);
        [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr hwnd, out Rect rect);
        [DllImport("user32.dll")] static extern IntPtr SendMessageW(IntPtr hwnd, uint msg, IntPtr wParam, IntPtr lParam);
        [DllImport("user32.dll", EntryPoint = "GetWindowLongPtrW")] static extern IntPtr GetWindowLongPtrW(IntPtr hwnd, int index);
        [DllImport("user32.dll", EntryPoint = "GetWindowLongW")] static extern IntPtr GetWindowLongW(IntPtr hwnd, int index);
        [DllImport("user32.dll")] static extern bool SetWindowDisplayAffinity(IntPtr hwnd, uint affinity);
        [DllImport("user32.dll")] static extern bool GetWindowDisplayAffinity(IntPtr hwnd, out uint affinity);
        [DllImport("user32.dll")] static extern IntPtr GetDC(IntPtr hwnd);
        [DllImport("user32.dll")] static extern int ReleaseDC(IntPtr hwnd, IntPtr dc);
        [DllImport("gdi32.dll")] static extern uint GetPixel(IntPtr dc, int x, int y);
        [DllImport("gdi32.dll")] static extern IntPtr CreateSolidBrush(uint color);
        [DllImport("user32.dll")] static extern IntPtr DefWindowProcW(IntPtr hwnd, uint msg, IntPtr wParam, IntPtr lParam);
        [DllImport("user32.dll", SetLastError = true)] static extern IntPtr SetWindowsHookEx(int id, HookProc proc, IntPtr module, uint thread);
        [DllImport("user32.dll")] static extern bool UnhookWindowsHookEx(IntPtr hook);
        [DllImport("user32.dll")] static extern IntPtr CallNextHookEx(IntPtr hook, int code, IntPtr wParam, IntPtr lParam);
        [DllImport("user32.dll")] static extern uint SendInput(uint count, byte[] inputs, int size);
        [DllImport("user32.dll")] static extern bool LockWorkStation();
        [DllImport("user32.dll")] static extern int GetMessage(out Msg msg, IntPtr hwnd, uint min, uint max);
        [DllImport("user32.dll")] static extern bool PeekMessage(out Msg msg, IntPtr hwnd, uint min, uint max, uint remove);
        [DllImport("user32.dll")] static extern bool TranslateMessage(ref Msg msg);
        [DllImport("user32.dll")] static extern IntPtr DispatchMessage(ref Msg msg);
        [DllImport("user32.dll")] static extern bool PostThreadMessage(uint thread, uint msg, IntPtr wParam, IntPtr lParam);
        [DllImport("user32.dll")] static extern IntPtr RegisterPowerSettingNotification(IntPtr hwnd, ref Guid setting, uint flags);
        [DllImport("kernel32.dll")] static extern uint GetCurrentThreadId();
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] static extern IntPtr GetModuleHandle(string name);
        [DllImport("kernel32.dll")] static extern uint SetThreadExecutionState(uint flags);
        [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
        [DllImport("kernel32.dll")] static extern bool GetSystemPowerStatus(out SystemPowerStatus status);
        [DllImport("powrprof.dll", SetLastError = true)] static extern IntPtr PowerCreateRequest(ref ReasonContext context);
        [DllImport("powrprof.dll")] static extern bool PowerSetRequest(IntPtr request, int type);
        [DllImport("powrprof.dll")] static extern bool PowerClearRequest(IntPtr request, int type);
        [DllImport("shell32.dll")] static extern bool IsUserAnAdmin();
        [DllImport("wtsapi32.dll", CharSet = CharSet.Unicode)] static extern bool WTSQuerySessionInformationW(IntPtr server, int session, int klass, out IntPtr buffer, out int bytes);
        [DllImport("wtsapi32.dll")] static extern void WTSFreeMemory(IntPtr memory);
        [DllImport("wtsapi32.dll")] static extern bool WTSRegisterSessionNotification(IntPtr hwnd, int flags);
        [DllImport("wtsapi32.dll")] static extern bool WTSUnRegisterSessionNotification(IntPtr hwnd);
    }
}
