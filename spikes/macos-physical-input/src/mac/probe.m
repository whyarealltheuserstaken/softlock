// Throwaway m05 listen-only tap. Returns every event unchanged.
// Releases the tap on every path. Does not lock the session or read serials.

#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>
#import <CoreGraphics/CoreGraphics.h>
#import <Foundation/Foundation.h>
#import <IOKit/hid/IOHIDKeys.h>
#import <IOKit/hid/IOHIDManager.h>
#import <IOKit/hid/IOHIDUsageTables.h>

#include <dlfcn.h>
#include <spawn.h>
#include <stdatomic.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/sysctl.h>
#include <sys/time.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

extern char **environ;

static const int kProbeKey = 0;
static const int kMarkerKey = 50;
static const int kSlowKey = 8;
static const int kSlowSleepMs = 2000;

@class ProbeWindow;
static ProbeWindow *g_window = nil;
static CFMachPortRef g_tap = NULL;
static CFRunLoopSourceRef g_tap_source = NULL;
static _Atomic int g_tap_released = 1;

static _Atomic int g_count_post = 0;
static _Atomic int64_t g_last_post_ms = 0;
static _Atomic int64_t g_wait_state = 0;
static _Atomic int64_t g_wait_pid = 0;
static _Atomic int64_t g_wait_type = 0;
static _Atomic int g_wait_saw = 0;
static _Atomic int64_t g_disable_type = 0;
static _Atomic int g_marker_seen = 0;
static _Atomic int64_t g_slow_entered_ms = 0;
static _Atomic int64_t g_slow_returned_ms = 0;
static _Atomic int64_t g_disable_ms = 0;

typedef struct {
    int saw;
    int64_t state_id;
    int64_t source_pid;
    int64_t event_type;
    int64_t disable_type;
    char error[256];
} WaitOut;

typedef struct {
    int keyboards;
    int pointing;
    int trackpad;
    int error;
} InventoryOut;

@interface ProbeWindow : NSWindow
@end
@implementation ProbeWindow
- (BOOL)canBecomeKeyWindow {
    return YES;
}
- (BOOL)canBecomeMainWindow {
    return YES;
}
@end

@interface ProbeView : NSView
@end
@implementation ProbeView
- (BOOL)acceptsFirstResponder {
    return YES;
}
@end

static int64_t now_ms(void) {
    struct timespec ts;
    clock_gettime(CLOCK_REALTIME, &ts);
    return (int64_t)ts.tv_sec * 1000 + ts.tv_nsec / 1000000;
}

static void run_loop(double seconds) {
    NSDate *until = [NSDate dateWithTimeIntervalSinceNow:seconds];
    while ([until timeIntervalSinceNow] > 0) {
        [[NSRunLoop currentRunLoop] runMode:NSDefaultRunLoopMode beforeDate:until];
    }
}

static void pump(double seconds) {
    run_loop(seconds);
}

static void join_path(char *out, size_t cap, const char *dir, const char *name) {
    snprintf(out, cap, "%s/%s", dir, name);
}

static void write_text(const char *path, const char *text) {
    FILE *file = fopen(path, "w");
    if (file == NULL) {
        return;
    }
    fputs(text, file);
    fclose(file);
}

static int read_int_line(const char *text, const char *key, long long *out) {
    const char *found = strstr(text, key);
    if (found == NULL) {
        return -1;
    }
    return sscanf(found + strlen(key), "%lld", out) == 1 ? 0 : -1;
}

int m05_hw_model(char *dest, size_t cap) {
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

static CGEventRef listen_callback(CGEventTapProxy proxy, CGEventType type, CGEventRef event, void *user) {
    (void)proxy;
    (void)user;
    if (type == kCGEventTapDisabledByTimeout || type == kCGEventTapDisabledByUserInput) {
        atomic_store(&g_disable_type, (int64_t)type);
        atomic_store(&g_disable_ms, now_ms());
        return NULL;
    }
    if (event == NULL) {
        return event;
    }
    atomic_store(&g_wait_state, CGEventGetIntegerValueField(event, kCGEventSourceStateID));
    atomic_store(&g_wait_pid, CGEventGetIntegerValueField(event, kCGEventSourceUnixProcessID));
    atomic_store(&g_wait_type, (int64_t)type);
    atomic_store(&g_wait_saw, 1);
    if (type == kCGEventKeyDown) {
        int64_t key = CGEventGetIntegerValueField(event, kCGKeyboardEventKeycode);
        if (key == kProbeKey) {
            atomic_fetch_add(&g_count_post, 1);
            atomic_store(&g_last_post_ms, now_ms());
        }
        if (key == kMarkerKey) {
            atomic_store(&g_marker_seen, 1);
        }
    }
    return event;
}

static void tap_release(void) {
    if (atomic_load(&g_tap_released) != 0) {
        return;
    }
    if (g_tap != NULL) {
        CGEventTapEnable(g_tap, false);
        if (g_tap_source != NULL) {
            CFRunLoopRemoveSource(CFRunLoopGetMain(), g_tap_source, kCFRunLoopCommonModes);
            CFRelease(g_tap_source);
            g_tap_source = NULL;
        }
        CFRelease(g_tap);
        g_tap = NULL;
    }
    atomic_store(&g_tap_released, 1);
}

int m05_tap_create(char *error, size_t cap) {
    if (error != NULL && cap > 0) {
        error[0] = 0;
    }
    tap_release();
    atomic_store(&g_wait_saw, 0);
    atomic_store(&g_disable_type, 0);
    CGEventMask mask = CGEventMaskBit(kCGEventKeyDown) | CGEventMaskBit(kCGEventLeftMouseDown) |
                       CGEventMaskBit(kCGEventRightMouseDown) | CGEventMaskBit(kCGEventOtherMouseDown) |
                       CGEventMaskBit(kCGEventScrollWheel);
    g_tap = CGEventTapCreate(kCGSessionEventTap, kCGHeadInsertEventTap, kCGEventTapOptionListenOnly, mask,
                             listen_callback, NULL);
    if (g_tap == NULL) {
        if (error != NULL) {
            snprintf(error, cap, "CGEventTapCreate returned NULL");
        }
        atomic_store(&g_tap_released, 1);
        return 0;
    }
    g_tap_source = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, g_tap, 0);
    CFRunLoopAddSource(CFRunLoopGetMain(), g_tap_source, kCFRunLoopCommonModes);
    CGEventTapEnable(g_tap, true);
    atomic_store(&g_tap_released, 0);
    return 1;
}

int m05_tap_release(void) {
    tap_release();
    return 1;
}

int m05_tap_released(void) {
    return atomic_load(&g_tap_released) != 0 ? 1 : 0;
}

int m05_clear_wait(void) {
    atomic_store(&g_wait_saw, 0);
    return 0;
}

int m05_wait_saw(void) {
    return atomic_load(&g_wait_saw);
}

int m05_last_event(WaitOut *out) {
    if (out == NULL) {
        return 2;
    }
    memset(out, 0, sizeof *out);
    out->saw = atomic_load(&g_wait_saw);
    out->state_id = atomic_load(&g_wait_state);
    out->source_pid = atomic_load(&g_wait_pid);
    out->event_type = atomic_load(&g_wait_type);
    out->disable_type = atomic_load(&g_disable_type);
    return 0;
}

int m05_tap_wait(const char *kind, int timeout_ms, WaitOut *out) {
    if (out == NULL) {
        return 2;
    }
    memset(out, 0, sizeof *out);
    if (g_tap == NULL) {
        snprintf(out->error, sizeof out->error, "no tap");
        return 1;
    }
    CGEventType want = kCGEventKeyDown;
    if (kind != NULL && strcmp(kind, "click") == 0) {
        want = kCGEventLeftMouseDown;
    } else if (kind != NULL && strcmp(kind, "scroll") == 0) {
        want = kCGEventScrollWheel;
    } else if (kind != NULL && strcmp(kind, "any-key") == 0) {
        want = kCGEventKeyDown;
    }
    atomic_store(&g_wait_saw, 0);
    NSDate *until = [NSDate dateWithTimeIntervalSinceNow:(timeout_ms / 1000.0)];
    while ([until timeIntervalSinceNow] > 0) {
        if (atomic_load(&g_wait_saw) != 0) {
            int64_t type = atomic_load(&g_wait_type);
            int match = 0;
            if (want == kCGEventLeftMouseDown) {
                match = type == kCGEventLeftMouseDown || type == kCGEventRightMouseDown ||
                        type == kCGEventOtherMouseDown;
            } else {
                match = type == (int64_t)want;
            }
            if (match) {
                out->saw = 1;
                out->state_id = atomic_load(&g_wait_state);
                out->source_pid = atomic_load(&g_wait_pid);
                out->event_type = type;
                out->disable_type = atomic_load(&g_disable_type);
                return 0;
            }
            atomic_store(&g_wait_saw, 0);
        }
        if (atomic_load(&g_disable_type) != 0) {
            out->disable_type = atomic_load(&g_disable_type);
            snprintf(out->error, sizeof out->error, "tap disabled");
            return 1;
        }
        run_loop(0.05);
    }
    out->disable_type = atomic_load(&g_disable_type);
    return 0;
}

int m05_last_disable(void) {
    return (int)atomic_load(&g_disable_type);
}

static int count_hid(uint32_t usage_page, uint32_t usage) {
    IOHIDManagerRef manager = IOHIDManagerCreate(kCFAllocatorDefault, kIOHIDOptionsTypeNone);
    if (manager == NULL) {
        return -1;
    }
    NSDictionary *filter = @{
        @(kIOHIDDeviceUsagePageKey) : @(usage_page),
        @(kIOHIDDeviceUsageKey) : @(usage),
    };
    IOHIDManagerSetDeviceMatching(manager, (__bridge CFDictionaryRef)filter);
    if (IOHIDManagerOpen(manager, kIOHIDOptionsTypeNone) != kIOReturnSuccess) {
        CFRelease(manager);
        return -1;
    }
    CFSetRef devices = IOHIDManagerCopyDevices(manager);
    int count = devices == NULL ? 0 : (int)CFSetGetCount(devices);
    if (devices != NULL) {
        CFRelease(devices);
    }
    IOHIDManagerClose(manager, kIOHIDOptionsTypeNone);
    CFRelease(manager);
    return count;
}

static int has_trackpad(void) {
    IOHIDManagerRef manager = IOHIDManagerCreate(kCFAllocatorDefault, kIOHIDOptionsTypeNone);
    if (manager == NULL) {
        return -1;
    }
    IOHIDManagerSetDeviceMatching(manager, NULL);
    if (IOHIDManagerOpen(manager, kIOHIDOptionsTypeNone) != kIOReturnSuccess) {
        CFRelease(manager);
        return -1;
    }
    CFSetRef devices = IOHIDManagerCopyDevices(manager);
    int found = 0;
    if (devices != NULL) {
        CFIndex count = CFSetGetCount(devices);
        const void **values = malloc(sizeof(void *) * (size_t)count);
        if (values != NULL) {
            CFSetGetValues(devices, values);
            for (CFIndex i = 0; i < count; i++) {
                IOHIDDeviceRef device = (IOHIDDeviceRef)values[i];
                CFTypeRef name = IOHIDDeviceGetProperty(device, CFSTR(kIOHIDProductKey));
                if (name != NULL && CFGetTypeID(name) == CFStringGetTypeID()) {
                    NSString *product = (__bridge NSString *)name;
                    if ([product localizedCaseInsensitiveContainsString:@"trackpad"]) {
                        found = 1;
                        break;
                    }
                }
            }
            free(values);
        }
        CFRelease(devices);
    }
    IOHIDManagerClose(manager, kIOHIDOptionsTypeNone);
    CFRelease(manager);
    return found;
}

int m05_hid_inventory(InventoryOut *out) {
    if (out == NULL) {
        return 2;
    }
    memset(out, 0, sizeof *out);
    out->keyboards = count_hid(kHIDPage_GenericDesktop, kHIDUsage_GD_Keyboard);
    out->pointing = count_hid(kHIDPage_GenericDesktop, kHIDUsage_GD_Mouse);
    out->trackpad = has_trackpad();
    if (out->keyboards < 0 || out->pointing < 0) {
        out->error = 1;
        out->keyboards = out->keyboards < 0 ? 0 : out->keyboards;
        out->pointing = out->pointing < 0 ? 0 : out->pointing;
        return 1;
    }
    return 0;
}

int m05_secure_enabled(void) {
    typedef Boolean (*Fn)(void);
    Fn fn = (Fn)dlsym(RTLD_DEFAULT, "IsSecureEventInputEnabled");
    if (fn == NULL) {
        return -1;
    }
    return fn() ? 1 : 0;
}

static void activate_app(void) {
    [NSApplication sharedApplication];
    [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];
    [NSApp activateIgnoringOtherApps:YES];
}

static BOOL wait_until_key(NSWindow *window) {
    NSDate *until = [NSDate dateWithTimeIntervalSinceNow:3];
    while ([until timeIntervalSinceNow] > 0) {
        if ([window isKeyWindow]) {
            return YES;
        }
        [window makeKeyAndOrderFront:nil];
        pump(0.05);
    }
    return [window isKeyWindow];
}

int m05_classify_target(const char *dir) {
    if (dir == NULL) {
        return 2;
    }
    @autoreleasepool {
        activate_app();
        NSScreen *screen = [NSScreen mainScreen];
        NSRect frame_rect = screen != nil ? screen.visibleFrame : NSMakeRect(0, 0, 800, 600);
        NSRect frame = NSMakeRect(NSMidX(frame_rect) - 160, NSMidY(frame_rect) - 120, 320, 240);
        ProbeWindow *window = [[ProbeWindow alloc] initWithContentRect:frame
                                                            styleMask:NSWindowStyleMaskBorderless
                                                              backing:NSBackingStoreBuffered
                                                                defer:NO];
        if (window == nil) {
            return 1;
        }
        window.releasedWhenClosed = NO;
        window.opaque = YES;
        window.level = NSNormalWindowLevel;
        window.backgroundColor = [NSColor colorWithSRGBRed:0.1 green:0.4 blue:0.8 alpha:1];
        ProbeView *view = [[ProbeView alloc] initWithFrame:NSMakeRect(0, 0, 320, 240)];
        window.contentView = view;
        g_window = window;
        [window orderFrontRegardless];
        [window makeKeyAndOrderFront:nil];
        wait_until_key(window);
        id monitor = [NSEvent addLocalMonitorForEventsMatchingMask:NSEventMaskKeyDown | NSEventMaskLeftMouseDown |
                                                                       NSEventMaskScrollWheel
                                                           handler:^NSEvent *(NSEvent *event) {
                                                             if (event.type == NSEventTypeKeyDown &&
                                                                 event.keyCode == kProbeKey) {
                                                                 atomic_fetch_add(&g_count_post, 1);
                                                                 atomic_store(&g_last_post_ms, now_ms());
                                                             }
                                                             if (event.type == NSEventTypeKeyDown ||
                                                                 event.type == NSEventTypeLeftMouseDown ||
                                                                 event.type == NSEventTypeScrollWheel) {
                                                                 char body[256];
                                                                 snprintf(body, sizeof body,
                                                                          "ready=1\ncount0=%d\nlast0=%lld\nkind=%ld\n",
                                                                          atomic_load(&g_count_post),
                                                                          (long long)atomic_load(&g_last_post_ms),
                                                                          (long)event.type);
                                                                 char path[1024];
                                                                 join_path(path, sizeof path, dir, "target-state.txt");
                                                                 write_text(path, body);
                                                             }
                                                             return event;
                                                           }];
        char stop[1024];
        join_path(stop, sizeof stop, dir, "target.stop");
        NSDate *until = [NSDate dateWithTimeIntervalSinceNow:90];
        while ([until timeIntervalSinceNow] > 0) {
            char body[256];
            snprintf(body, sizeof body, "ready=1\ncount0=%d\nlast0=%lld\n", atomic_load(&g_count_post),
                     (long long)atomic_load(&g_last_post_ms));
            char path[1024];
            join_path(path, sizeof path, dir, "target-state.txt");
            write_text(path, body);
            if (access(stop, F_OK) == 0) {
                break;
            }
            pump(0.05);
        }
        if (monitor != nil) {
            [NSEvent removeMonitor:monitor];
        }
        if (g_window != nil) {
            [g_window orderOut:nil];
            g_window = nil;
        }
    }
    return 0;
}

static int state_from_name(const char *name, CGEventSourceStateID *state) {
    if (strcmp(name, "kCGEventSourceStatePrivate") == 0) {
        *state = kCGEventSourceStatePrivate;
        return 0;
    }
    if (strcmp(name, "kCGEventSourceStateCombinedSessionState") == 0) {
        *state = kCGEventSourceStateCombinedSessionState;
        return 0;
    }
    if (strcmp(name, "kCGEventSourceStateHIDSystemState") == 0) {
        *state = kCGEventSourceStateHIDSystemState;
        return 0;
    }
    return -1;
}

static int post_key_code(CGEventSourceRef source, CGKeyCode key, int64_t *state_id, int64_t *source_pid, char *error,
                         size_t cap) {
    CGEventRef down = CGEventCreateKeyboardEvent(source, key, true);
    CGEventRef up = CGEventCreateKeyboardEvent(source, key, false);
    if (down == NULL || up == NULL) {
        snprintf(error, cap, "CGEventCreateKeyboardEvent returned NULL");
        if (down != NULL) {
            CFRelease(down);
        }
        if (up != NULL) {
            CFRelease(up);
        }
        return -1;
    }
    *state_id = CGEventGetIntegerValueField(down, kCGEventSourceStateID);
    *source_pid = CGEventGetIntegerValueField(down, kCGEventSourceUnixProcessID);
    CGEventPost(kCGSessionEventTap, down);
    CGEventPost(kCGSessionEventTap, up);
    CFRelease(down);
    CFRelease(up);
    return 0;
}

int m05_classify_post(const char *source, const char *dir) {
    if (source == NULL || dir == NULL) {
        return 2;
    }
    char error[256];
    error[0] = 0;
    CGEventSourceStateID state;
    if (state_from_name(source, &state) != 0) {
        snprintf(error, sizeof error, "unknown CGEventSourceStateID %s", source);
        return 1;
    }
    CGEventSourceRef created = CGEventSourceCreate(state);
    if (created == NULL) {
        snprintf(error, sizeof error, "CGEventSourceCreate returned NULL");
        return 1;
    }
    int64_t state_id = 0;
    int64_t source_pid = 0;
    int rc = post_key_code(created, (CGKeyCode)kProbeKey, &state_id, &source_pid, error, sizeof error);
    CFRelease(created);
    char body[256];
    if (rc != 0) {
        snprintf(body, sizeof body, "ok=0\nstate_id=0\nsource_pid=0\nposter_pid=%d\nerror=%s\n", (int)getpid(), error);
    } else {
        snprintf(body, sizeof body, "ok=1\nstate_id=%lld\nsource_pid=%lld\nposter_pid=%d\n", (long long)state_id,
                 (long long)source_pid, (int)getpid());
    }
    char path[1024];
    join_path(path, sizeof path, dir, "last-post.txt");
    write_text(path, body);
    return rc == 0 ? 0 : 1;
}

int m05_read_target_count(const char *dir) {
    char path[1024];
    join_path(path, sizeof path, dir, "target-state.txt");
    FILE *file = fopen(path, "r");
    if (file == NULL) {
        return -1;
    }
    char text[512];
    size_t n = fread(text, 1, sizeof text - 1, file);
    fclose(file);
    text[n] = 0;
    long long ready = 0;
    long long c0 = 0;
    if (read_int_line(text, "ready=", &ready) != 0 || ready != 1) {
        return -1;
    }
    read_int_line(text, "count0=", &c0);
    return (int)c0;
}

int m05_pump(int millis) {
    pump(millis / 1000.0);
    return 0;
}

int m05_post_named(const char *source, const char *dir) {
    return m05_classify_post(source, dir);
}
