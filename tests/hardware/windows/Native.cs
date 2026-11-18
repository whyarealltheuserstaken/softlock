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
        const int MarkerSize = 32;
        const uint LwaColorKey = 0x00000001;
        const uint LwaAlpha = 0x00000002;
        const uint WmPaint = 0x000F;
        const uint WmEraseBkgnd = 0x0014;
        const uint WmNcHitTest = 0x0084;
        const uint WmPowerBroadcast = 0x0218;
        const uint WmDeviceChange = 0x0219;
        const uint WmDisplayChange = 0x007E;
        const uint WmWtsSessionChange = 0x02B1;
        const int DbtDeviceArrival = 0x8000;
        const int DbtDeviceRemoveComplete = 0x8004;
        const int DbtDevtypDeviceInterface = 5;
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
        static bool paintMarker;
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
        static int displayAdds;
        static int displayRemoves;
        static int displayChanges;
        static int lastActive = -1;
        static IntPtr monitorNotify = IntPtr.Zero;
        static int sessionLocks;
        static int sessionUnlocks;
        static IntPtr powerRequest = IntPtr.Zero;
        static IntPtr reasonText = IntPtr.Zero;
        static bool dpiReady;
        static ConsoleCtrlHandler ctrlHandler;
        static int interrupted;

        public static string DetectJson()
        {
            try
            {
                Census census = ReadCensus();
                int protocol = ClientProtocol();
                byte ac = 255;
                SystemPowerStatus power;
                if (GetSystemPowerStatus(out power))
                {
                    ac = power.ACLineStatus;
                }
                bool admin = IsUserAnAdmin();
                string internalJson = census.internalCount < 0 ? "null" : (census.internalCount > 0 ? "true" : "false");
                return "{\"ok\":true,\"monitors\":" + DisplayCount(census)
                    + ",\"enumerated\":" + census.enumerated
                    + ",\"active\":" + census.active
                    + ",\"configured\":" + census.configured
                    + ",\"internal_count\":" + census.internalCount
                    + ",\"external_count\":" + census.externalCount
                    + ",\"protocol\":" + protocol
                    + ",\"ac\":" + ac
                    + ",\"internal\":" + internalJson
                    + ",\"elevated\":" + (admin ? "true" : "false") + "}";
            }
            catch (Exception error)
            {
                return "{\"ok\":false,\"monitors\":0,\"enumerated\":-1,\"active\":-1,\"configured\":-1,\"internal_count\":-1,\"external_count\":-1,\"protocol\":-1,\"ac\":255,\"internal\":null,\"elevated\":false,\"error\":\"" + Esc(error.Message) + "\"}";
            }
        }

        // DISPLAYCONFIG_PATH_INFO is 72 bytes. The source is 20 bytes, so the
        // target LUID is at 20, outputTechnology at 36, and targetAvailable at 60.
        // https://learn.microsoft.com/en-us/windows/win32/api/wingdi/ns-wingdi-displayconfig_path_info
        class DisplayTarget
        {
            public int luidLow;
            public int luidHigh;
            public int id;
            public int tech;
            public bool available;
            public string Key { get { return luidLow + ":" + luidHigh + ":" + id; } }
        }

        class Census
        {
            public int attached = -1;
            public int active = -1;
            public int configured = -1;
            public int internalCount = -1;
            public int externalCount = -1;
            public int enumerated = -1;
        }

        static bool IsInternal(int tech)
        {
            if ((tech & unchecked((int)0x80000000)) != 0) return true;
            return tech == 6 || tech == 11 || tech == 13;
        }

        static Census ReadCensus()
        {
            Census census = new Census();
            try
            {
                EnsureDpi();
                census.enumerated = MonitorRects().Count;
            }
            catch (Exception)
            {
                census.enumerated = -1;
            }
            List<DisplayTarget> active = QueryTargets(2, true);
            List<DisplayTarget> all = QueryTargets(1, false);
            if (active != null)
            {
                census.active = Distinct(active).Count;
            }
            List<DisplayTarget> present = null;
            if (all != null)
            {
                List<DisplayTarget> unique = Distinct(all);
                census.configured = unique.Count;
                present = new List<DisplayTarget>();
                foreach (DisplayTarget target in unique)
                {
                    if (target.available) present.Add(target);
                }
            }
            if (active != null)
            {
                if (present == null) present = new List<DisplayTarget>();
                foreach (DisplayTarget target in Distinct(active))
                {
                    bool known = false;
                    foreach (DisplayTarget have in present)
                    {
                        if (have.Key == target.Key) { known = true; break; }
                    }
                    if (!known) present.Add(target);
                }
            }
            if (present != null)
            {
                census.attached = present.Count;
                int internals = 0;
                foreach (DisplayTarget target in present)
                {
                    if (IsInternal(target.tech)) internals++;
                }
                census.internalCount = internals;
                census.externalCount = present.Count - internals;
            }
            return census;
        }

        static int DisplayCount(Census census)
        {
            if (census.attached >= 0) return census.attached;
            if (census.active >= 0) return census.active;
            return census.enumerated;
        }

        static List<DisplayTarget> Distinct(List<DisplayTarget> targets)
        {
            List<DisplayTarget> unique = new List<DisplayTarget>();
            foreach (DisplayTarget target in targets)
            {
                bool known = false;
                foreach (DisplayTarget have in unique)
                {
                    if (have.Key == target.Key) { known = true; break; }
                }
                if (!known) unique.Add(target);
            }
            return unique;
        }

        static List<DisplayTarget> QueryTargets(uint flags, bool forceAvailable)
        {
            for (int attempt = 0; attempt < 4; attempt++)
            {
                uint pathCount;
                uint modeCount;
                if (GetDisplayConfigBufferSizes(flags, out pathCount, out modeCount) != 0)
                {
                    return null;
                }
                if (pathCount == 0)
                {
                    return new List<DisplayTarget>();
                }
                int modeBytes = (int)modeCount * 128;
                if (modeBytes < 128) modeBytes = 128;
                IntPtr paths = Marshal.AllocHGlobal((int)pathCount * 128);
                IntPtr modes = Marshal.AllocHGlobal(modeBytes);
                try
                {
                    uint pathsOut = pathCount;
                    uint modesOut = modeCount;
                    int rc = QueryDisplayConfig(flags, ref pathsOut, paths, ref modesOut, modes, IntPtr.Zero);
                    if (rc == 122) continue;
                    if (rc != 0) return null;
                    List<DisplayTarget> found = new List<DisplayTarget>();
                    for (int i = 0; i < (int)pathsOut; i++)
                    {
                        int at = i * 72;
                        DisplayTarget target = new DisplayTarget();
                        target.luidLow = Marshal.ReadInt32(paths, at + 20);
                        target.luidHigh = Marshal.ReadInt32(paths, at + 24);
                        target.id = Marshal.ReadInt32(paths, at + 28);
                        target.tech = Marshal.ReadInt32(paths, at + 36);
                        target.available = forceAvailable || Marshal.ReadInt32(paths, at + 60) != 0;
                        found.Add(target);
                    }
                    return found;
                }
                finally
                {
                    Marshal.FreeHGlobal(paths);
                    Marshal.FreeHGlobal(modes);
                }
            }
            return null;
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
                    return "{\"ok\":true,\"monitors\":0,\"windows\":0,\"topmost\":false,\"click_through\":false,\"covered\":false,\"affinity\":false,\"marker_visible\":false,\"captured\":false}";
                }
                paintMarker = false;
                overlayProc = OverlayWnd;
                IntPtr proc = Marshal.GetFunctionPointerForDelegate(overlayProc);
                IntPtr brush = GetStockObject(4);
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
                bool markerVisible = true;
                bool captured = false;
                int painted = 0;
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
                        markerVisible = false;
                        continue;
                    }
                    windows.Add(hwnd);
                    // Stay hidden through the first paint. Black is the color key, so
                    // only the 32x32 marker is visible once the window is shown.
                    paintMarker = false;
                    SetLayeredWindowAttributes(hwnd, 0, 255, LwaColorKey | LwaAlpha);
                    UpdateWindow(hwnd);
                    paintMarker = true;
                    InvalidateRect(hwnd, IntPtr.Zero, false);
                    UpdateWindow(hwnd);
                    ShowWindow(hwnd, 8);
                    FlushFrame();
                    painted++;
                    int cx = monitor.left + (width / 2);
                    int cy = monitor.top + (height / 2);
                    if (ScreenPixel(cx, cy) != Marker)
                    {
                        markerVisible = false;
                    }
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
                    FlushFrame();
                    if (ScreenPixel(cx, cy) == Marker)
                    {
                        captured = true;
                    }
                }
                if (painted == 0)
                {
                    markerVisible = false;
                }
                return "{\"ok\":true,\"monitors\":" + monitors.Count
                    + ",\"windows\":" + windows.Count
                    + ",\"topmost\":" + Bool(topmost)
                    + ",\"click_through\":" + Bool(click)
                    + ",\"covered\":" + Bool(covered)
                    + ",\"affinity\":" + Bool(affinity)
                    + ",\"marker_visible\":" + Bool(markerVisible)
                    + ",\"captured\":" + Bool(captured) + "}";
            }
            catch (Exception error)
            {
                return "{\"ok\":false,\"monitors\":0,\"windows\":0,\"topmost\":false,\"click_through\":false,\"covered\":false,\"affinity\":false,\"marker_visible\":false,\"captured\":false,\"error\":\"" + Esc(error.Message) + "\"}";
            }
            finally
            {
                paintMarker = false;
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
                reasonText = Marshal.StringToHGlobalUni("hardware suite");
                ReasonContext context = new ReasonContext();
                context.Version = 0;
                context.Flags = 0x1;
                context.SimpleReasonString = reasonText;
                powerRequest = PowerCreateRequest(ref context);
                if (powerRequest == IntPtr.Zero)
                {
                    return "{\"ok\":false,\"api\":false,\"system\":false,\"display\":false,\"system_error\":" + Marshal.GetLastWin32Error() + ",\"display_error\":0,\"error\":\"PowerCreateRequest\"}";
                }
                int systemRc = PowerSetRequest(powerRequest, 1);
                int systemError = systemRc != 0 ? 0 : Marshal.GetLastWin32Error();
                int displayRc = PowerSetRequest(powerRequest, 0);
                int displayError = displayRc != 0 ? 0 : Marshal.GetLastWin32Error();
                int executionRc = PowerSetRequest(powerRequest, 3);
                bool system = systemRc != 0;
                return "{\"ok\":" + Bool(system) + ",\"api\":" + Bool(system) + ",\"system\":" + Bool(system) + ",\"display\":" + Bool(displayRc != 0) + ",\"execution\":" + Bool(executionRc != 0) + ",\"system_error\":" + systemError + ",\"display_error\":" + displayError + ",\"error\":\"\"}";
            }
            catch (Exception error)
            {
                return "{\"ok\":false,\"api\":false,\"system\":false,\"display\":false,\"system_error\":0,\"display_error\":0,\"error\":\"" + Esc(error.Message) + "\"}";
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

        public static string HotplugJson()
        {
            Census census = ReadCensus();
            lock (Gate)
            {
                return "{\"active\":" + DisplayCount(census)
                    + ",\"monitors\":" + census.enumerated
                    + ",\"internal\":" + census.internalCount
                    + ",\"external\":" + census.externalCount
                    + ",\"adds\":" + displayAdds
                    + ",\"removes\":" + displayRemoves
                    + ",\"changes\":" + displayChanges + "}";
            }
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

        public static void InstallCtrlHandler()
        {
            ctrlHandler = OnCtrl;
            SetConsoleCtrlHandler(ctrlHandler, true);
        }

        public static bool WasInterrupted()
        {
            return System.Threading.Interlocked.CompareExchange(ref interrupted, 0, 0) != 0;
        }

        // WTSINFOEXW.Level is a DWORD, then padding to the 8-byte union.
        // WTSINFOEX_LEVEL1_W.SessionFlags is the third DWORD of that union (offset 16).
        // https://learn.microsoft.com/en-us/windows/win32/api/wtsapi32/ns-wtsapi32-wtsinfoex_level1_w
        // WTS_SESSIONSTATE_LOCK is 0 and WTS_SESSIONSTATE_UNLOCK is 1 since Windows 7.
        public static string SessionStateJson()
        {
            IntPtr buffer;
            int bytes;
            if (!WTSQuerySessionInformationW(IntPtr.Zero, -1, 25, out buffer, out bytes) || buffer == IntPtr.Zero)
            {
                return "{\"ok\":false,\"locked\":false,\"flags\":-1}";
            }
            try
            {
                if (bytes < 20)
                {
                    return "{\"ok\":false,\"locked\":false,\"flags\":-1}";
                }
                int level = Marshal.ReadInt32(buffer);
                int flags = Marshal.ReadInt32(buffer, 16);
                bool known = level == 1 && (flags == 0 || flags == 1);
                bool locked = known && flags == 0;
                return "{\"ok\":" + Bool(known) + ",\"locked\":" + Bool(locked) + ",\"flags\":" + flags + "}";
            }
            finally
            {
                WTSFreeMemory(buffer);
            }
        }

        static bool OnCtrl(int ctrlType)
        {
            if (ctrlType == 0 || ctrlType == 1)
            {
                System.Threading.Interlocked.Exchange(ref interrupted, 1);
                return true;
            }
            return false;
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
                Guid lid = LidGuid;
                Guid display = DisplayGuid;
                RegisterPowerSettingNotification(listenerHwnd, ref lid, 0);
                RegisterPowerSettingNotification(listenerHwnd, ref display, 0);
                monitorNotify = RegisterMonitorNotify(listenerHwnd);
            }
            ListenerReady.Set();
            Msg msg;
            while (GetMessage(out msg, IntPtr.Zero, 0, 0) > 0)
            {
                TranslateMessage(ref msg);
                DispatchMessage(ref msg);
            }
            if (monitorNotify != IntPtr.Zero)
            {
                UnregisterDeviceNotification(monitorNotify);
                monitorNotify = IntPtr.Zero;
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
            else if (message == WmDisplayChange)
            {
                NoteDisplayChange();
            }
            else if (message == WmDeviceChange)
            {
                int kind = wParam.ToInt32();
                if (kind == DbtDeviceArrival)
                {
                    NoteDevice(true);
                }
                else if (kind == DbtDeviceRemoveComplete)
                {
                    NoteDevice(false);
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

        static void NoteDisplayChange()
        {
            int active = DisplayCount(ReadCensus());
            lock (Gate)
            {
                displayChanges++;
                if (lastActive >= 0 && active >= 0)
                {
                    if (active > lastActive) { displayAdds++; }
                    else if (active < lastActive) { displayRemoves++; }
                }
                if (active >= 0) { lastActive = active; }
            }
        }

        static void NoteDevice(bool arrived)
        {
            int active = DisplayCount(ReadCensus());
            lock (Gate)
            {
                displayChanges++;
                if (lastActive >= 0 && active >= 0 && active != lastActive)
                {
                    if (arrived) { displayAdds++; } else { displayRemoves++; }
                }
                if (active >= 0) { lastActive = active; }
            }
        }

        static IntPtr RegisterMonitorNotify(IntPtr hwnd)
        {
            DevBroadcastDeviceInterface filter = new DevBroadcastDeviceInterface();
            filter.size = Marshal.SizeOf(typeof(DevBroadcastDeviceInterface));
            filter.deviceType = DbtDevtypDeviceInterface;
            filter.classGuid = new Guid("E6F07B5F-EE97-4a90-B076-33F57BF4EAA7");
            IntPtr memory = Marshal.AllocHGlobal(filter.size);
            try
            {
                Marshal.StructureToPtr(filter, memory, false);
                return RegisterDeviceNotificationW(hwnd, memory, 0);
            }
            finally
            {
                Marshal.FreeHGlobal(memory);
            }
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
            if (message == WmPaint)
            {
                PaintStruct paint;
                IntPtr hdc = BeginPaint(hwnd, out paint);
                if (hdc != IntPtr.Zero)
                {
                    Rect client;
                    if (!GetClientRect(hwnd, out client))
                    {
                        client = paint.area;
                    }
                    IntPtr black = CreateSolidBrush(0);
                    FillRect(hdc, ref client, black);
                    DeleteObject(black);
                    if (paintMarker)
                    {
                        Rect patch = MarkerPatch(client);
                        IntPtr brush = CreateSolidBrush(Marker);
                        FillRect(hdc, ref patch, brush);
                        DeleteObject(brush);
                    }
                }
                EndPaint(hwnd, ref paint);
                return IntPtr.Zero;
            }
            if (message == WmNcHitTest)
            {
                return new IntPtr(-1);
            }
            if (message == WmEraseBkgnd)
            {
                return new IntPtr(1);
            }
            return DefWindowProcW(hwnd, message, wParam, lParam);
        }

        static Rect MarkerPatch(Rect client)
        {
            int width = client.right - client.left;
            int height = client.bottom - client.top;
            int size = MarkerSize;
            if (size > width) { size = width; }
            if (size > height) { size = height; }
            Rect patch = new Rect();
            patch.left = client.left + ((width - size) / 2);
            patch.top = client.top + ((height - size) / 2);
            patch.right = patch.left + size;
            patch.bottom = patch.top + size;
            return patch;
        }

        static uint ScreenPixel(int x, int y)
        {
            IntPtr dc = GetDC(IntPtr.Zero);
            if (dc == IntPtr.Zero)
            {
                return 0xFFFFFFFF;
            }
            uint pixel = GetPixel(dc, x, y);
            ReleaseDC(IntPtr.Zero, dc);
            return pixel;
        }

        static void FlushFrame()
        {
            try
            {
                DwmFlush();
            }
            catch (Exception)
            {
            }
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

        delegate bool ConsoleCtrlHandler(int ctrlType);
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

        [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
        struct DevBroadcastDeviceInterface
        {
            public int size;
            public int deviceType;
            public int reserved;
            public Guid classGuid;
            public short name;
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
        struct PaintStruct
        {
            public IntPtr hdc;
            public int erase;
            public Rect area;
            public int restore;
            public int incUpdate;
            public int reserved1;
            public int reserved2;
            public int reserved3;
            public int reserved4;
            public int reserved5;
            public int reserved6;
            public int reserved7;
            public int reserved8;
            public int pad;
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

        [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr RegisterDeviceNotificationW(IntPtr hwnd, IntPtr filter, uint flags);
        [DllImport("user32.dll")] static extern bool UnregisterDeviceNotification(IntPtr handle);
        [DllImport("user32.dll")] static extern int GetDisplayConfigBufferSizes(uint flags, out uint pathCount, out uint modeCount);
        [DllImport("user32.dll")] static extern int QueryDisplayConfig(uint flags, ref uint pathCount, IntPtr paths, ref uint modeCount, IntPtr modes, IntPtr topology);
        [DllImport("user32.dll")] static extern bool SetProcessDPIAware();
        [DllImport("user32.dll")] static extern bool EnumDisplayMonitors(IntPtr hdc, IntPtr clip, MonitorEnum proc, IntPtr data);
        [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)] static extern ushort RegisterClassW(ref WndClass wc);
        [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        static extern IntPtr CreateWindowExW(uint ex, string klass, string name, uint style, int x, int y, int w, int h, IntPtr parent, IntPtr menu, IntPtr instance, IntPtr param);
        [DllImport("user32.dll")] static extern bool DestroyWindow(IntPtr hwnd);
        [DllImport("user32.dll")] static extern bool ShowWindow(IntPtr hwnd, int cmd);
        [DllImport("user32.dll")] static extern bool UpdateWindow(IntPtr hwnd);
        [DllImport("user32.dll")] static extern bool InvalidateRect(IntPtr hwnd, IntPtr rect, bool erase);
        [DllImport("user32.dll")] static extern IntPtr BeginPaint(IntPtr hwnd, out PaintStruct paint);
        [DllImport("user32.dll")] static extern bool EndPaint(IntPtr hwnd, ref PaintStruct paint);
        [DllImport("user32.dll")] static extern bool GetClientRect(IntPtr hwnd, out Rect rect);
        [DllImport("user32.dll")] static extern int FillRect(IntPtr hdc, ref Rect rect, IntPtr brush);
        [DllImport("gdi32.dll")] static extern IntPtr CreateSolidBrush(uint color);
        [DllImport("gdi32.dll")] static extern bool DeleteObject(IntPtr obj);
        [DllImport("dwmapi.dll")] static extern int DwmFlush();
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
        [DllImport("gdi32.dll")] static extern IntPtr GetStockObject(int obj);
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
        [DllImport("kernel32.dll")] static extern bool SetConsoleCtrlHandler(ConsoleCtrlHandler handler, bool add);
        [DllImport("kernel32.dll")] static extern uint GetCurrentThreadId();
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] static extern IntPtr GetModuleHandle(string name);
        [DllImport("kernel32.dll")] static extern uint SetThreadExecutionState(uint flags);
        [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
        [DllImport("kernel32.dll")] static extern bool GetSystemPowerStatus(out SystemPowerStatus status);
        [DllImport("kernel32.dll", SetLastError = true)] static extern IntPtr PowerCreateRequest(ref ReasonContext context);
        [DllImport("kernel32.dll", SetLastError = true)] static extern int PowerSetRequest(IntPtr request, int type);
        [DllImport("kernel32.dll", SetLastError = true)] static extern int PowerClearRequest(IntPtr request, int type);
        [DllImport("shell32.dll")] static extern bool IsUserAnAdmin();
        [DllImport("wtsapi32.dll", CharSet = CharSet.Unicode)] static extern bool WTSQuerySessionInformationW(IntPtr server, int session, int klass, out IntPtr buffer, out int bytes);
        [DllImport("wtsapi32.dll")] static extern void WTSFreeMemory(IntPtr memory);
        [DllImport("wtsapi32.dll")] static extern bool WTSRegisterSessionNotification(IntPtr hwnd, int flags);
        [DllImport("wtsapi32.dll")] static extern bool WTSUnRegisterSessionNotification(IntPtr hwnd);
    }
}
