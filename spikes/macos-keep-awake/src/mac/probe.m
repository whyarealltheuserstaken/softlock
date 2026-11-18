// Throwaway m07 keep-awake kit. Does not lock the session.
// Does not read hardware identifiers. Overlay ignores the mouse and is not key.

#import <AppKit/AppKit.h>
#import <Foundation/Foundation.h>
#import <LocalAuthentication/LocalAuthentication.h>
#import <IOKit/IOKitLib.h>
#import <IOKit/IOMessage.h>
#import <IOKit/pwr_mgt/IOPMLib.h>

#include <stdatomic.h>
#include <stdbool.h>
#include <stdio.h>
#include <string.h>
#include <sys/sysctl.h>
#include <unistd.h>

typedef struct {
    int prevent_user_idle_system_sleep;
    int prevent_user_idle_display_sleep;
    int no_idle_sleep;
    int no_display_sleep;
} NamesOut;

typedef struct {
    int created;
    int released;
    uint32_t system_id;
    uint32_t display_id;
    char reason[128];
    char error[256];
} AssertOut;

typedef struct {
    int ordered_front;
    int display_count;
    int opaque;
    double frame_x;
    double frame_y;
    double frame_w;
    double frame_h;
    char error[256];
} WindowOut;

typedef struct {
    uint64_t ticks;
    int occlusion_visible;
    char error[256];
} TimerOut;

static NSMutableArray<NSWindow *> *g_overlay = nil;
static NSWindow *g_timer = nil;
static NSTextField *g_timer_label = nil;
static uint64_t g_ticks = 0;
static atomic_int g_escape = 0;
static id g_escape_monitor = nil;

static void pump(NSTimeInterval seconds) {
    if (seconds <= 0) {
        return;
    }
    NSDate *until = [NSDate dateWithTimeIntervalSinceNow:seconds];
    while ([until timeIntervalSinceNow] > 0) {
        NSTimeInterval remaining = [until timeIntervalSinceNow];
        NSTimeInterval slice = remaining < 0.01 ? remaining : 0.01;
        if (slice <= 0) {
            break;
        }
        NSEvent *event = [NSApp nextEventMatchingMask:NSEventMaskAny
                                            untilDate:[NSDate dateWithTimeIntervalSinceNow:slice]
                                               inMode:NSDefaultRunLoopMode
                                              dequeue:YES];
        if (event) {
            [NSApp sendEvent:event];
        }
    }
}

static void ensure_app(void) {
    NSApplication *app = [NSApplication sharedApplication];
    [app setActivationPolicy:NSApplicationActivationPolicyAccessory];
}

@interface CoverWindow : NSWindow
@end
@implementation CoverWindow
- (BOOL)canBecomeKeyWindow {
    return NO;
}
- (BOOL)canBecomeMainWindow {
    return NO;
}
@end

int m07_assertion_names(NamesOut *out) {
    if (out == NULL) {
        return 1;
    }
    memset(out, 0, sizeof *out);
#ifdef kIOPMAssertionTypePreventUserIdleSystemSleep
    out->prevent_user_idle_system_sleep = 1;
#endif
#ifdef kIOPMAssertionTypePreventUserIdleDisplaySleep
    out->prevent_user_idle_display_sleep = 1;
#endif
#ifdef kIOPMAssertionTypeNoIdleSleep
    out->no_idle_sleep = 1;
#endif
#ifdef kIOPMAssertionTypeNoDisplaySleep
    out->no_display_sleep = 1;
#endif
    return 0;
}

int m07_assertions_create(const char *reason, AssertOut *out) {
    if (out == NULL) {
        return 1;
    }
    memset(out, 0, sizeof *out);
    if (reason == NULL || reason[0] == 0) {
        snprintf(out->error, sizeof out->error, "reason string is empty");
        return 0;
    }
    snprintf(out->reason, sizeof out->reason, "%s", reason);
    CFStringRef name = CFStringCreateWithCString(kCFAllocatorDefault, reason, kCFStringEncodingUTF8);
    if (name == NULL) {
        snprintf(out->error, sizeof out->error, "CFStringCreateWithCString returned NULL");
        return 0;
    }
    IOPMAssertionID system_id = 0;
    IOPMAssertionID display_id = 0;
    IOReturn system_rc = IOPMAssertionCreateWithName(
        kIOPMAssertionTypePreventUserIdleSystemSleep, kIOPMAssertionLevelOn, name, &system_id);
    IOReturn display_rc = IOPMAssertionCreateWithName(
        kIOPMAssertionTypePreventUserIdleDisplaySleep, kIOPMAssertionLevelOn, name, &display_id);
    CFRelease(name);
    if (system_rc != kIOReturnSuccess || display_rc != kIOReturnSuccess) {
        snprintf(out->error, sizeof out->error, "IOPMAssertionCreateWithName system=%d display=%d",
                 system_rc, display_rc);
        if (system_id != 0) {
            IOPMAssertionRelease(system_id);
        }
        if (display_id != 0) {
            IOPMAssertionRelease(display_id);
        }
        return 0;
    }
    out->created = 1;
    out->system_id = system_id;
    out->display_id = display_id;
    return 0;
}

int m07_assertions_release(uint32_t system_id, uint32_t display_id) {
    IOReturn system_rc = kIOReturnSuccess;
    IOReturn display_rc = kIOReturnSuccess;
    if (system_id != 0) {
        system_rc = IOPMAssertionRelease(system_id);
    }
    if (display_id != 0) {
        display_rc = IOPMAssertionRelease(display_id);
    }
    return (system_rc == kIOReturnSuccess && display_rc == kIOReturnSuccess) ? 0 : 1;
}

#ifndef kIOMainPortDefault
#define kIOMainPortDefault kIOMasterPortDefault
#endif

static int service_present(const char *name) {
    io_iterator_t iter = IO_OBJECT_NULL;
    kern_return_t kr =
        IOServiceGetMatchingServices(kIOMainPortDefault, IOServiceMatching(name), &iter);
    if (kr != KERN_SUCCESS) {
        return -1;
    }
    io_object_t found = IOIteratorNext(iter);
    int present = found == IO_OBJECT_NULL ? 0 : 1;
    if (found != IO_OBJECT_NULL) {
        IOObjectRelease(found);
    }
    IOObjectRelease(iter);
    return present;
}

int m07_touch_id(void) {
    @autoreleasepool {
        LAContext *context = [[LAContext alloc] init];
        if (context != nil) {
            NSError *error = nil;
            (void)[context canEvaluatePolicy:LAPolicyDeviceOwnerAuthenticationWithBiometrics
                                       error:&error];
            if (context.biometryType == LABiometryTypeTouchID) {
                return 1;
            }
            if (context.biometryType == LABiometryTypeNone) {
                return 0;
            }
            return 0;
        }
    }
    return service_present("AppleBiometricSensor");
}

// 1 closed, 0 open, -1 the clamshell property is absent.
int m07_clamshell_closed(void) {
    char path[128];
    snprintf(path, sizeof path, "%s:/IOPowerConnection/IOPMrootDomain", kIOPowerPlane);
    io_registry_entry_t root = IORegistryEntryFromPath(kIOMainPortDefault, path);
    if (root == IO_OBJECT_NULL) {
        return -1;
    }
    CFTypeRef value = IORegistryEntryCreateCFProperty(root, CFSTR("AppleClamshellState"),
                                                      kCFAllocatorDefault, 0);
    IOObjectRelease(root);
    if (value == NULL) {
        return -1;
    }
    int closed = -1;
    if (CFGetTypeID(value) == CFBooleanGetTypeID()) {
        closed = CFBooleanGetValue((CFBooleanRef)value) ? 1 : 0;
    }
    CFRelease(value);
    return closed;
}

// 1 when a clamshell property or an internal battery exists, 0 when the registry says neither.
int m07_has_lid(void) {
    int clamshell = m07_clamshell_closed();
    if (clamshell == 0 || clamshell == 1) {
        return 1;
    }
    return service_present("AppleSmartBattery");
}

int m07_display_count(void) {
    @autoreleasepool {
        ensure_app();
        return (int)[NSScreen screens].count;
    }
}

int m07_hw_model(char *dest, size_t cap) {
    if (dest == NULL || cap == 0) {
        return 1;
    }
    memset(dest, 0, cap);
    size_t size = cap;
    if (sysctlbyname("hw.model", dest, &size, NULL, 0) != 0) {
        dest[0] = 0;
        return 1;
    }
    dest[cap - 1] = 0;
    return 0;
}

static void install_escape_monitor(void) {
    atomic_store(&g_escape, 0);
    if (g_escape_monitor != nil) {
        [NSEvent removeMonitor:g_escape_monitor];
        g_escape_monitor = nil;
    }
    g_escape_monitor =
        [NSEvent addGlobalMonitorForEventsMatchingMask:NSEventMaskKeyDown
                                               handler:^(NSEvent *event) {
                                                 if (event.keyCode == 53) {
                                                     atomic_store(&g_escape, 1);
                                                 }
                                               }];
}

static CoverWindow *make_cover(NSRect frame) {
    CoverWindow *window = [[CoverWindow alloc] initWithContentRect:frame
                                                        styleMask:NSWindowStyleMaskBorderless
                                                          backing:NSBackingStoreBuffered
                                                            defer:NO];
    window.releasedWhenClosed = NO;
    window.opaque = YES;
    window.alphaValue = 1.0;
    window.hasShadow = NO;
    window.ignoresMouseEvents = YES;
    window.level = NSStatusWindowLevel;
    window.backgroundColor = [NSColor blackColor];
    window.collectionBehavior = NSWindowCollectionBehaviorCanJoinAllSpaces |
                                NSWindowCollectionBehaviorFullScreenAuxiliary;
    [window orderFrontRegardless];
    return window;
}

static int overlay_is_opaque(void) {
    if (g_overlay.count == 0) {
        return 0;
    }
    for (NSWindow *window in g_overlay) {
        if (!window.isOpaque || window.alphaValue < 0.999) {
            return 0;
        }
    }
    return 1;
}

int m07_overlay_open(WindowOut *out, double x, double y, double w, double h, int use_rect) {
    if (out == NULL) {
        return 1;
    }
    memset(out, 0, sizeof *out);
    @autoreleasepool {
        ensure_app();
        g_overlay = [NSMutableArray array];
        if (use_rect && w > 1 && h > 1) {
            CoverWindow *window = make_cover(NSMakeRect(x, y, w, h));
            out->display_count = (int)[NSScreen screens].count;
            out->frame_x = NSMinX(window.frame);
            out->frame_y = NSMinY(window.frame);
            out->frame_w = NSWidth(window.frame);
            out->frame_h = NSHeight(window.frame);
            [g_overlay addObject:window];
        } else {
            NSArray<NSScreen *> *screens = [NSScreen screens];
            out->display_count = (int)screens.count;
            for (NSScreen *screen in screens) {
                [g_overlay addObject:make_cover(screen.frame)];
            }
        }
        install_escape_monitor();
        pump(0.2);
        BOOL front = g_overlay.count > 0;
        for (NSWindow *window in g_overlay) {
            if (!window.isVisible) {
                front = NO;
            }
        }
        out->ordered_front = front ? 1 : 0;
        out->opaque = overlay_is_opaque();
        if (!front) {
            snprintf(out->error, sizeof out->error, "overlay windows were not visible");
        }
    }
    return 0;
}

int m07_overlay_close(void) {
    @autoreleasepool {
        if (g_escape_monitor != nil) {
            [NSEvent removeMonitor:g_escape_monitor];
            g_escape_monitor = nil;
        }
        for (NSWindow *window in g_overlay) {
            [window orderOut:nil];
            [window close];
        }
        g_overlay = nil;
    }
    return 0;
}

int m07_overlay_escape_seen(void) {
    return atomic_load(&g_escape);
}

void m07_overlay_pump(uint32_t millis) {
    @autoreleasepool {
        pump(millis / 1000.0);
    }
}

int m07_timer_open(WindowOut *out) {
    if (out == NULL) {
        return 1;
    }
    memset(out, 0, sizeof *out);
    g_ticks = 0;
    @autoreleasepool {
        ensure_app();
        NSScreen *screen = [NSScreen mainScreen];
        NSRect visible = screen != nil ? screen.visibleFrame : NSMakeRect(40, 40, 800, 600);
        NSRect frame = NSMakeRect(NSMinX(visible) + 40, NSMinY(visible) + 40, 280, 140);
        NSWindow *window = [[NSWindow alloc] initWithContentRect:frame
                                                      styleMask:NSWindowStyleMaskTitled
                                                        backing:NSBackingStoreBuffered
                                                          defer:NO];
        window.releasedWhenClosed = NO;
        window.title = @"m07 timer";
        window.level = NSNormalWindowLevel;
        NSTextField *label = [[NSTextField alloc] initWithFrame:NSMakeRect(16, 48, 248, 40)];
        label.editable = NO;
        label.bezeled = NO;
        label.drawsBackground = NO;
        label.stringValue = @"0";
        window.contentView = [[NSView alloc] initWithFrame:NSMakeRect(0, 0, 280, 140)];
        [window.contentView addSubview:label];
        [window orderFrontRegardless];
        g_timer = window;
        g_timer_label = label;
        out->display_count = (int)[NSScreen screens].count;
        out->frame_x = NSMinX(window.frame);
        out->frame_y = NSMinY(window.frame);
        out->frame_w = NSWidth(window.frame);
        out->frame_h = NSHeight(window.frame);
        out->ordered_front = window.isVisible ? 1 : 0;
        if (!window.isVisible) {
            snprintf(out->error, sizeof out->error, "timer window was not visible");
        }
    }
    return 0;
}

int m07_timer_close(void) {
    @autoreleasepool {
        [g_timer orderOut:nil];
        [g_timer close];
        g_timer = nil;
        g_timer_label = nil;
    }
    return 0;
}

static IONotificationPortRef g_power_notify = NULL;
static io_connect_t g_power_root = 0;
static io_object_t g_power_object = 0;
static id g_ws_will_sleep = nil;
static id g_ws_did_wake = nil;
static id g_ws_screens_sleep = nil;
static id g_ws_screens_wake = nil;
static atomic_int g_sys_sleep = 0;
static atomic_int g_sys_wake = 0;
static atomic_int g_disp_sleep = 0;
static atomic_int g_disp_wake = 0;

static void power_interest(void *refcon, io_service_t service, uint32_t messageType,
                           void *messageArgument) {
    (void)refcon;
    (void)service;
    switch (messageType) {
    case kIOMessageCanSystemSleep:
        if (g_power_root != 0) {
            IOAllowPowerChange(g_power_root, (long)messageArgument);
        }
        break;
    case kIOMessageSystemWillSleep:
        atomic_store(&g_sys_sleep, 1);
        if (g_power_root != 0) {
            IOAllowPowerChange(g_power_root, (long)messageArgument);
        }
        break;
    case kIOMessageSystemHasPoweredOn:
        atomic_store(&g_sys_wake, 1);
        break;
    default:
        break;
    }
}

void m07_power_reset(void) {
    atomic_store(&g_sys_sleep, 0);
    atomic_store(&g_sys_wake, 0);
    atomic_store(&g_disp_sleep, 0);
    atomic_store(&g_disp_wake, 0);
}

int m07_power_watch_start(void) {
    @autoreleasepool {
        ensure_app();
        m07_power_reset();
        if (g_power_root == 0) {
            g_power_root =
                IORegisterForSystemPower(NULL, &g_power_notify, power_interest, &g_power_object);
            if (g_power_root != 0 && g_power_notify != NULL) {
                CFRunLoopAddSource(CFRunLoopGetCurrent(),
                                   IONotificationPortGetRunLoopSource(g_power_notify),
                                   kCFRunLoopDefaultMode);
            }
        }
        NSNotificationCenter *center = [NSWorkspace sharedWorkspace].notificationCenter;
        if (g_ws_will_sleep == nil) {
            g_ws_will_sleep = [center addObserverForName:NSWorkspaceWillSleepNotification
                                                  object:nil
                                                   queue:nil
                                              usingBlock:^(NSNotification *note) {
                                                (void)note;
                                                atomic_store(&g_sys_sleep, 1);
                                              }];
        }
        if (g_ws_did_wake == nil) {
            g_ws_did_wake = [center addObserverForName:NSWorkspaceDidWakeNotification
                                                object:nil
                                                 queue:nil
                                            usingBlock:^(NSNotification *note) {
                                              (void)note;
                                              atomic_store(&g_sys_wake, 1);
                                            }];
        }
        if (g_ws_screens_sleep == nil) {
            g_ws_screens_sleep =
                [center addObserverForName:NSWorkspaceScreensDidSleepNotification
                                    object:nil
                                     queue:nil
                                usingBlock:^(NSNotification *note) {
                                  (void)note;
                                  atomic_store(&g_disp_sleep, 1);
                                }];
        }
        if (g_ws_screens_wake == nil) {
            g_ws_screens_wake =
                [center addObserverForName:NSWorkspaceScreensDidWakeNotification
                                    object:nil
                                     queue:nil
                                usingBlock:^(NSNotification *note) {
                                  (void)note;
                                  atomic_store(&g_disp_wake, 1);
                                }];
        }
    }
    return (g_power_root != 0) ? 0 : 1;
}

int m07_power_watch_stop(void) {
    @autoreleasepool {
        NSNotificationCenter *center = [NSWorkspace sharedWorkspace].notificationCenter;
        if (g_ws_will_sleep != nil) {
            [center removeObserver:g_ws_will_sleep];
            g_ws_will_sleep = nil;
        }
        if (g_ws_did_wake != nil) {
            [center removeObserver:g_ws_did_wake];
            g_ws_did_wake = nil;
        }
        if (g_ws_screens_sleep != nil) {
            [center removeObserver:g_ws_screens_sleep];
            g_ws_screens_sleep = nil;
        }
        if (g_ws_screens_wake != nil) {
            [center removeObserver:g_ws_screens_wake];
            g_ws_screens_wake = nil;
        }
        if (g_power_notify != NULL) {
            CFRunLoopRemoveSource(CFRunLoopGetCurrent(),
                                  IONotificationPortGetRunLoopSource(g_power_notify),
                                  kCFRunLoopDefaultMode);
        }
        if (g_power_root != 0 && g_power_object != 0) {
            IODeregisterForSystemPower(&g_power_object);
        }
        if (g_power_notify != NULL) {
            IONotificationPortDestroy(g_power_notify);
            g_power_notify = NULL;
        }
        g_power_root = 0;
        g_power_object = 0;
    }
    return 0;
}

int m07_power_system_slept(void) { return atomic_load(&g_sys_sleep); }
int m07_power_system_woke(void) { return atomic_load(&g_sys_wake); }
int m07_power_display_slept(void) { return atomic_load(&g_disp_sleep); }
int m07_power_display_woke(void) { return atomic_load(&g_disp_wake); }

void m07_power_pump(uint32_t millis) { m07_overlay_pump(millis); }

int m07_timer_tick(TimerOut *out) {
    if (out == NULL) {
        return 1;
    }
    memset(out, 0, sizeof *out);
    @autoreleasepool {
        g_ticks += 1;
        out->ticks = g_ticks;
        g_timer_label.stringValue = [NSString stringWithFormat:@"%llu", (unsigned long long)g_ticks];
        pump(0.01);
        BOOL visible = g_timer != nil && (g_timer.occlusionState & NSWindowOcclusionStateVisible) != 0;
        out->occlusion_visible = visible ? 1 : 0;
    }
    return 0;
}
