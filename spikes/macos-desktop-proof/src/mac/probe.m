// Throwaway desktop probe. Listen-only taps return the event unchanged.
// This file does not lock the session and does not read hardware identifiers.

#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>
#import <CoreGraphics/CoreGraphics.h>
#import <Foundation/Foundation.h>
#import <CoreMedia/CoreMedia.h>
#import <CoreVideo/CoreVideo.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>

#include <Security/AuthSession.h>
#include <ctype.h>
#include <dlfcn.h>
#include <pthread.h>
#include <signal.h>
#include <spawn.h>
#include <stdatomic.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/time.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

extern char **environ;

typedef struct {
    int ordered_front;
    int key_window;
    int key_received;
    char post_location[64];
    char error[768];
    char stage[64];
} WindowOut;

typedef struct {
    int created;
    int callback_fired;
    int accessibility_trusted;
    int listen_preflight;
    int prompt;
    char detail[768];
} TapOut;

typedef struct {
    int opened;
    int width;
    int height;
    int nonblack;
    int marker;
    int control;
    int prompt;
    char detail[768];
} CaptureOut;

typedef struct {
    int created;
    int deleted;
    int listen_saw;
    int control_saw;
    int accessibility_trusted;
    int listen_preflight;
    int prompt;
    char detail[768];
} ActiveOut;

typedef struct {
    int uid;
    int pid;
    int graphic;
    int status;
    char error[256];
} SessionOut;

static const int kProbeKey = 0;
@class ProbeWindow;
static ProbeWindow *g_window = nil;
static NSWindow *g_sharing = nil;
static NSWindow *g_control = nil;

static void stage_named(const char *name) {
    struct timeval tv;
    gettimeofday(&tv, NULL);
    fprintf(stderr, "stage=%s t=%lld.%03d\n", name, (long long)tv.tv_sec, (int)(tv.tv_usec / 1000));
    fflush(stderr);
}

static void stage_line(WindowOut *out, const char *name) {
    if (out != NULL) {
        snprintf(out->stage, sizeof out->stage, "%s", name);
    }
    stage_named(name);
}

static volatile int g_key_received = 0;

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

static void run_loop(NSTimeInterval seconds) {
    CFRunLoopRunInMode(kCFRunLoopDefaultMode, seconds, true);
}

static void pump(NSTimeInterval seconds) {
    NSDate *until = [NSDate dateWithTimeIntervalSinceNow:seconds];
    while ([until timeIntervalSinceNow] > 0) {
        NSEvent *event = [NSApp nextEventMatchingMask:NSEventMaskAny
                                             untilDate:[NSDate dateWithTimeIntervalSinceNow:0.05]
                                                inMode:NSDefaultRunLoopMode
                                               dequeue:YES];
        if (event) {
            [NSApp sendEvent:event];
        }
    }
}

static int contains_ci(const char *haystack, const char *needle) {
    if (haystack == NULL || needle == NULL || needle[0] == 0) {
        return 0;
    }
    size_t needle_len = strlen(needle);
    for (size_t start = 0; haystack[start] != 0; start++) {
        size_t index = 0;
        while (index < needle_len && haystack[start + index] != 0 &&
               tolower((unsigned char)haystack[start + index]) ==
                   tolower((unsigned char)needle[index])) {
            index++;
        }
        if (index == needle_len) {
            return 1;
        }
    }
    return 0;
}

static int classify_window(const char *owner, const char *name) {
    if (contains_ci(name, "Input Monitoring")) {
        return 2;
    }
    if (contains_ci(name, "Screen Recording") || contains_ci(name, "ScreenCapture")) {
        return 3;
    }
    if (contains_ci(name, "Accessibility")) {
        return 1;
    }
    if (strcasestr(owner, "universalAccessAuthWarn") != NULL) {
        return 1;
    }
    if (strcmp(owner, "tccd") == 0) {
        return 4;
    }
    return 0;
}

static void copy_cf_string(CFDictionaryRef info, CFStringRef key, char *dest, size_t cap) {
    if (cap == 0) {
        return;
    }
    dest[0] = 0;
    CFTypeRef value = CFDictionaryGetValue(info, key);
    if (value == NULL || CFGetTypeID(value) != CFStringGetTypeID()) {
        return;
    }
    CFStringGetCString((CFStringRef)value, dest, (CFIndex)cap, kCFStringEncodingUTF8);
}

int macos_prompt_code(void) {
    CFArrayRef list = CGWindowListCopyWindowInfo(kCGWindowListOptionOnScreenOnly, kCGNullWindowID);
    if (list == NULL) {
        return 0;
    }
    int code = 0;
    CFIndex count = CFArrayGetCount(list);
    if (count > 200) {
        count = 200;
    }
    for (CFIndex index = 0; index < count && code == 0; index++) {
        CFDictionaryRef info = CFArrayGetValueAtIndex(list, index);
        if (info == NULL || CFGetTypeID(info) != CFDictionaryGetTypeID()) {
            continue;
        }
        char owner[128];
        char name[256];
        copy_cf_string(info, kCGWindowOwnerName, owner, sizeof owner);
        copy_cf_string(info, kCGWindowName, name, sizeof name);
        code = classify_window(owner, name);
    }
    CFRelease(list);
    return code;
}

static bool preflight_listen(int *value) {
    typedef bool (*PreflightFn)(void);
    PreflightFn function = (PreflightFn)dlsym(RTLD_DEFAULT, "CGPreflightListenEventAccess");
    if (function == NULL) {
        *value = -1;
        return false;
    }
    *value = function() ? 1 : 0;
    return true;
}

static const char *post_preflight(void) {
    typedef bool (*PreflightFn)(void);
    PreflightFn function = (PreflightFn)dlsym(RTLD_DEFAULT, "CGPreflightPostEventAccess");
    if (function == NULL) {
        return "unknown";
    }
    return function() ? "true" : "false";
}

static int post_key(CGEventSourceRef source, CGEventTapLocation location, char *error, size_t cap) {
    CGEventRef down = CGEventCreateKeyboardEvent(source, (CGKeyCode)kProbeKey, true);
    CGEventRef up = CGEventCreateKeyboardEvent(source, (CGKeyCode)kProbeKey, false);
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
    CGEventPost(location, down);
    CGEventPost(location, up);
    CFRelease(down);
    CFRelease(up);
    return 0;
}

static void activate_app(void) {
    NSApplication *app = [NSApplication sharedApplication];
    [app setActivationPolicy:NSApplicationActivationPolicyRegular];
    // [NSApp activate] on macOS 14+ does not take focus from another app.
    NSRunningApplication *current = [NSRunningApplication currentApplication];
    [current activateWithOptions:NSApplicationActivateIgnoringOtherApps];
    [app activateIgnoringOtherApps:YES];
}

static BOOL wait_until_key(ProbeWindow *window) {
    NSDate *until = [NSDate dateWithTimeIntervalSinceNow:2.0];
    while ([until timeIntervalSinceNow] > 0) {
        if (NSApp.isActive && window.isKeyWindow) {
            return YES;
        }
        [window makeKeyAndOrderFront:nil];
        activate_app();
        pump(0.05);
    }
    return NSApp.isActive && window.isKeyWindow;
}

int macos_window_open(WindowOut *out) {
    if (out == NULL) {
        return 1;
    }
    memset(out, 0, sizeof *out);
    g_key_received = 0;
    @autoreleasepool {
        @try {
            activate_app();
            NSScreen *screen = [NSScreen mainScreen];
            NSRect frame_rect = screen != nil ? screen.visibleFrame : NSMakeRect(0, 0, 800, 600);
            NSRect frame = NSMakeRect(NSMidX(frame_rect) - 160, NSMidY(frame_rect) - 120, 320, 240);
            ProbeWindow *window = [[ProbeWindow alloc] initWithContentRect:frame
                                                                  styleMask:NSWindowStyleMaskBorderless
                                                                    backing:NSBackingStoreBuffered
                                                                      defer:NO];
            if (window == nil) {
                snprintf(out->error, sizeof out->error, "NSWindow initWithContentRect returned nil");
                return 0;
            }
            // releasedWhenClosed defaults to YES. Under ARC, [window close] then
            // releases the same object again when the local goes out of scope.
            window.releasedWhenClosed = NO;
            window.opaque = YES;
            window.hasShadow = NO;
            window.level = NSNormalWindowLevel;
            window.backgroundColor = [NSColor colorWithSRGBRed:0.86 green:0.04 blue:0.63 alpha:1];
            ProbeView *view = [[ProbeView alloc] initWithFrame:NSMakeRect(0, 0, 320, 240)];
            window.contentView = view;
            g_window = window;
            stage_line(out, "order-front");
            [window orderFrontRegardless];
            [window makeKeyAndOrderFront:nil];
            stage_line(out, "activate");
            BOOL became_key = wait_until_key(window);
            stage_line(out, "activate-done");
            stage_line(out, "order-front-done");

            BOOL on_screen = window.isVisible && window.screen != nil;
            BOOL unoccluded = (window.occlusionState & NSWindowOcclusionStateVisible) != 0;
            out->ordered_front = (on_screen && unoccluded) ? 1 : 0;
            out->key_window = became_key ? 1 : 0;
            if (window.screen == nil) {
                snprintf(out->error, sizeof out->error, "NSWindow screen is nil (no Aqua session)");
            } else if (out->ordered_front == 0) {
                snprintf(out->error, sizeof out->error,
                         "NSWindow orderFrontRegardless did not leave the window visible");
            }

            id monitor = [NSEvent addLocalMonitorForEventsMatchingMask:NSEventMaskKeyDown
                                                                handler:^NSEvent *(NSEvent *event) {
                                                                  if ((int)event.keyCode == kProbeKey) {
                                                                      g_key_received = 1;
                                                                  }
                                                                  return event;
                                                                }];
            stage_line(out, "CGEventPost");
            CGEventSourceRef source = CGEventSourceCreate(kCGEventSourceStateCombinedSessionState);
            if (source == NULL) {
                snprintf(out->error, sizeof out->error, "CGEventSourceCreate returned NULL");
            } else {
                struct {
                    CGEventTapLocation location;
                    const char *name;
                } attempts[3] = {
                    {kCGSessionEventTap, "kCGSessionEventTap"},
                    {kCGAnnotatedSessionEventTap, "kCGAnnotatedSessionEventTap"},
                    {kCGHIDEventTap, "kCGHIDEventTap"},
                };
                for (int index = 0; index < 3 && g_key_received == 0; index++) {
                    char post_error[256];
                    post_error[0] = 0;
                    if (post_key(source, attempts[index].location, post_error, sizeof post_error) != 0) {
                        snprintf(out->error, sizeof out->error, "%s", post_error);
                        break;
                    }
                    pump(0.4);
                    if (g_key_received != 0) {
                        snprintf(out->post_location, sizeof out->post_location, "%s", attempts[index].name);
                        out->error[0] = 0;
                    }
                }
                out->key_window = window.isKeyWindow ? 1 : 0;
                if (g_key_received == 0 && out->error[0] == 0 && out->key_window == 0) {
                    snprintf(out->error, sizeof out->error,
                             "CGEventPost reaches the session event stream (taps saw keyCode %d); "
                             "the window was not key (activation did not make it key), so the app did not receive it",
                             kProbeKey);
                } else if (g_key_received == 0 && out->error[0] == 0) {
                    snprintf(out->error, sizeof out->error,
                             "the window was key and CGEventPost did not deliver keyCode %d to the local monitor; "
                             "CGPreflightPostEventAccess=%s",
                             kProbeKey, post_preflight());
                }
                CFRelease(source);
            }
            stage_line(out, "CGEventPost-done");
            stage_line(out, "key-received");
            BOOL on_screen_after = window.isVisible && window.screen != nil;
            BOOL unoccluded_after = (window.occlusionState & NSWindowOcclusionStateVisible) != 0;
            out->ordered_front = (on_screen_after && unoccluded_after) ? 1 : 0;
            out->key_window = window.isKeyWindow ? 1 : 0;
            out->key_received = g_key_received != 0 ? 1 : 0;
            stage_line(out, "key-received-done");
            if (monitor != nil) {
                [NSEvent removeMonitor:monitor];
            }
        } @catch (NSException *exception) {
            const char *name = exception.name.UTF8String != NULL ? exception.name.UTF8String : "NSException";
            snprintf(out->error, sizeof out->error, "NSApplication exception %s", name);
        }
    }
    return 0;
}

typedef struct {
    int ordered_front;
    int ignores_mouse_events;
    int shielding_called;
    int window_area;
    char level[64];
    char collection[128];
    char sharing_default[64];
    char display_image_api[128];
    char error[768];
} SharingWindowOut;

typedef struct {
    int visible;
    int occlusion;
    int listed;
    char detail[256];
} SharingPresenceOut;

static const char *sharing_name(NSWindowSharingType type) {
    switch (type) {
    case NSWindowSharingNone:
        return "NSWindowSharingNone";
    case NSWindowSharingReadOnly:
        return "NSWindowSharingReadOnly";
    case NSWindowSharingReadWrite:
        return "NSWindowSharingReadWrite";
    default:
        return "other";
    }
}

static void sharing_collection(NSWindow *window, char *buf, size_t len) {
    NSWindowCollectionBehavior bits = window.collectionBehavior;
    buf[0] = 0;
    if ((bits & NSWindowCollectionBehaviorCanJoinAllSpaces) != 0) {
        strncat(buf, "canJoinAllSpaces", len - strlen(buf) - 1);
    }
    if ((bits & NSWindowCollectionBehaviorFullScreenAuxiliary) != 0) {
        if (buf[0] != 0) {
            strncat(buf, ",", len - strlen(buf) - 1);
        }
        strncat(buf, "fullScreenAuxiliary", len - strlen(buf) - 1);
    }
}

// Borderless marker window at NSScreenSaverWindowLevel. sharingType is left at
// AppKit's default. This function does not call CGShieldingWindowLevel().
int macos_sharing_open(SharingWindowOut *out) {
    if (out == NULL) {
        return 1;
    }
    memset(out, 0, sizeof *out);
    out->shielding_called = 0;
    @autoreleasepool {
        @try {
            activate_app();
            NSScreen *screen = [NSScreen mainScreen];
            NSRect frame_rect = screen != nil ? screen.frame : NSMakeRect(0, 0, 800, 600);
            NSRect frame = NSMakeRect(NSMidX(frame_rect) - 328, NSMidY(frame_rect) - 120, 320, 240);
            NSWindow *window = [[NSWindow alloc] initWithContentRect:frame
                                                            styleMask:NSWindowStyleMaskBorderless
                                                              backing:NSBackingStoreBuffered
                                                                defer:NO];
            if (window == nil) {
                snprintf(out->error, sizeof out->error, "NSWindow initWithContentRect returned nil");
                return 0;
            }
            window.releasedWhenClosed = NO;
            window.opaque = YES;
            window.hasShadow = NO;
            window.level = NSScreenSaverWindowLevel;
            window.collectionBehavior =
                NSWindowCollectionBehaviorCanJoinAllSpaces | NSWindowCollectionBehaviorFullScreenAuxiliary;
            window.ignoresMouseEvents = YES;
            window.backgroundColor = [NSColor colorWithSRGBRed:0.86 green:0.04 blue:0.63 alpha:1];
            ProbeView *view = [[ProbeView alloc] initWithFrame:NSMakeRect(0, 0, 320, 240)];
            view.wantsLayer = NO;
            window.contentView = view;
            g_sharing = window;
            [window orderFrontRegardless];
            pump(0.4);
            if (window.level == NSScreenSaverWindowLevel) {
                snprintf(out->level, sizeof out->level, "screenSaver");
            } else {
                snprintf(out->level, sizeof out->level, "level-%d", (int)window.level);
            }
            sharing_collection(window, out->collection, sizeof out->collection);
            out->ignores_mouse_events = window.ignoresMouseEvents ? 1 : 0;
            snprintf(out->sharing_default, sizeof out->sharing_default, "%s", sharing_name(window.sharingType));
            out->window_area = 320 * 240;
            out->display_image_api[0] = 0;
            if (dlsym(RTLD_DEFAULT, "CGWindowListCreateImage") != NULL) {
                strncat(out->display_image_api, "CGWindowListCreateImage", sizeof out->display_image_api - 1);
            }
            if (dlsym(RTLD_DEFAULT, "CGDisplayCreateImage") != NULL) {
                if (out->display_image_api[0] != 0) {
                    strncat(out->display_image_api, ",", sizeof out->display_image_api - strlen(out->display_image_api) - 1);
                }
                strncat(out->display_image_api, "CGDisplayCreateImage",
                        sizeof out->display_image_api - strlen(out->display_image_api) - 1);
            }
            BOOL on_screen = window.isVisible && window.screen != nil;
            BOOL unoccluded = (window.occlusionState & NSWindowOcclusionStateVisible) != 0;
            out->ordered_front = (on_screen && unoccluded) ? 1 : 0;
            if (out->ordered_front == 0) {
                snprintf(out->error, sizeof out->error,
                         "NSWindow orderFrontRegardless did not leave the screenSaver window visible");
            }
        } @catch (NSException *exception) {
            const char *name = exception.name.UTF8String != NULL ? exception.name.UTF8String : "NSException";
            snprintf(out->error, sizeof out->error, "NSWindow exception %s", name);
        }
    }
    return 0;
}

int macos_sharing_set_none(char *buf, size_t len) {
    if (buf == NULL || len == 0) {
        return 1;
    }
    buf[0] = 0;
    @autoreleasepool {
        if (g_sharing == nil) {
            snprintf(buf, len, "missing");
            return 0;
        }
        g_sharing.sharingType = NSWindowSharingNone;
        pump(0.4);
        snprintf(buf, len, "%s", sharing_name(g_sharing.sharingType));
    }
    return 0;
}

static NSWindow *make_color_window(NSRect frame, NSColor *color) {
    NSWindow *window = [[NSWindow alloc] initWithContentRect:frame
                                                    styleMask:NSWindowStyleMaskBorderless
                                                      backing:NSBackingStoreBuffered
                                                        defer:NO];
    if (window == nil) {
        return nil;
    }
    window.releasedWhenClosed = NO;
    window.opaque = YES;
    window.hasShadow = NO;
    window.level = NSScreenSaverWindowLevel;
    window.collectionBehavior =
        NSWindowCollectionBehaviorCanJoinAllSpaces | NSWindowCollectionBehaviorFullScreenAuxiliary;
    window.ignoresMouseEvents = YES;
    window.backgroundColor = color;
    ProbeView *view = [[ProbeView alloc] initWithFrame:NSMakeRect(0, 0, frame.size.width, frame.size.height)];
    window.contentView = view;
    return window;
}

int macos_sharing_control_open(void) {
    @autoreleasepool {
        if (g_control != nil) {
            return 0;
        }
        NSScreen *screen = [NSScreen mainScreen];
        NSRect frame_rect = screen != nil ? screen.frame : NSMakeRect(0, 0, 1024, 768);
        NSRect frame = NSMakeRect(NSMidX(frame_rect) + 8, NSMidY(frame_rect) - 120, 320, 240);
        NSWindow *window = make_color_window(frame, [NSColor colorWithSRGBRed:0.05 green:0.82 blue:0.08 alpha:1]);
        if (window == nil) {
            return 1;
        }
        g_control = window;
        [window orderFrontRegardless];
        pump(0.3);
    }
    return 0;
}

int macos_sharing_presence(SharingPresenceOut *out) {
    if (out == NULL) {
        return 1;
    }
    memset(out, 0, sizeof *out);
    @autoreleasepool {
        if (g_sharing == nil) {
            snprintf(out->detail, sizeof out->detail, "NSWindow sharing window is nil");
            return 0;
        }
        pump(0.2);
        out->visible = g_sharing.isVisible ? 1 : 0;
        out->occlusion = (g_sharing.occlusionState & NSWindowOcclusionStateVisible) != 0 ? 1 : 0;
        CGWindowID wanted = (CGWindowID)g_sharing.windowNumber;
        CFArrayRef list = CGWindowListCopyWindowInfo(kCGWindowListOptionOnScreenOnly, kCGNullWindowID);
        if (list == NULL) {
            snprintf(out->detail, sizeof out->detail, "CGWindowListCopyWindowInfo returned NULL");
            return 0;
        }
        CFIndex count = CFArrayGetCount(list);
        for (CFIndex index = 0; index < count; index++) {
            CFDictionaryRef info = CFArrayGetValueAtIndex(list, index);
            CFNumberRef number = CFDictionaryGetValue(info, kCGWindowNumber);
            int value = 0;
            if (number != NULL && CFNumberGetValue(number, kCFNumberIntType, &value) && (CGWindowID)value == wanted) {
                out->listed = 1;
                break;
            }
        }
        CFRelease(list);
        if (out->visible == 0 || out->occlusion == 0 || out->listed == 0) {
            snprintf(out->detail, sizeof out->detail,
                     "NSWindow visible=%d occlusion=%d CGWindowListCopyWindowInfo listed=%d number=%d",
                     out->visible, out->occlusion, out->listed, (int)wanted);
        }
    }
    return 0;
}

int macos_sharing_order_out(void) {
    stage_named("sharing-order-out");
    @autoreleasepool {
        if (g_sharing != nil) {
            [g_sharing orderOut:nil];
        }
        if (g_control != nil) {
            [g_control orderOut:nil];
        }
        pump(0.3);
    }
    return 0;
}

int macos_sharing_close(void) {
    stage_named("sharing-close");
    @autoreleasepool {
        if (g_sharing != nil) {
            g_sharing.releasedWhenClosed = NO;
            [g_sharing orderOut:nil];
            g_sharing = nil;
        }
        if (g_control != nil) {
            g_control.releasedWhenClosed = NO;
            [g_control orderOut:nil];
            g_control = nil;
        }
    }
    return 0;
}

int macos_window_close(void) {
    fprintf(stderr, "stage=order-out\n");
    fflush(stderr);
    @autoreleasepool {
        if (g_window != nil) {
            g_window.releasedWhenClosed = NO;
            [g_window orderOut:nil];
            g_window = nil;
        }
    }
    fprintf(stderr, "stage=order-out-done\n");
    fflush(stderr);
    return 0;
}

int macos_prove_window(WindowOut *out) {
    int rc = macos_window_open(out);
    macos_window_close();
    return rc;
}

static _Atomic int g_tap_fired = 0;

static CGEventRef tap_callback(CGEventTapProxy proxy, CGEventType type, CGEventRef event, void *user) {
    (void)proxy;
    (void)user;
    if (type == kCGEventKeyDown) {
        int64_t key = CGEventGetIntegerValueField(event, kCGKeyboardEventKeycode);
        if (key == kProbeKey) {
            atomic_store(&g_tap_fired, 1);
        }
    }
    return event;
}

int macos_prove_tap(const char *location, TapOut *out) {
    if (out == NULL || location == NULL) {
        return 1;
    }
    memset(out, 0, sizeof *out);
    out->accessibility_trusted = -1;
    out->listen_preflight = -1;
    CGEventTapLocation tap_location;
    if (strcmp(location, "kCGSessionEventTap") == 0) {
        tap_location = kCGSessionEventTap;
    } else if (strcmp(location, "kCGHIDEventTap") == 0) {
        tap_location = kCGHIDEventTap;
    } else {
        snprintf(out->detail, sizeof out->detail, "unknown tap location");
        return 1;
    }
    atomic_store(&g_tap_fired, 0);
    CFMachPortRef port = CGEventTapCreate(tap_location, kCGHeadInsertEventTap, kCGEventTapOptionListenOnly,
                                          CGEventMaskBit(kCGEventKeyDown), tap_callback, NULL);
    out->accessibility_trusted = AXIsProcessTrusted() ? 1 : 0;
    preflight_listen(&out->listen_preflight);
    out->prompt = macos_prompt_code();
    if (port == NULL) {
        out->created = 0;
        return 0;
    }
    out->created = 1;
    CFRunLoopSourceRef source = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, port, 0);
    if (source != NULL) {
        CFRunLoopAddSource(CFRunLoopGetCurrent(), source, kCFRunLoopCommonModes);
    }
    CGEventTapEnable(port, true);
    CGEventSourceRef event_source = CGEventSourceCreate(kCGEventSourceStateCombinedSessionState);
    if (event_source != NULL) {
        char ignored[64];
        post_key(event_source, tap_location, ignored, sizeof ignored);
        CFRelease(event_source);
    }
    NSDate *until = [NSDate dateWithTimeIntervalSinceNow:0.4];
    while (atomic_load(&g_tap_fired) == 0 && [until timeIntervalSinceNow] > 0) {
        CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.05, true);
    }
    out->callback_fired = atomic_load(&g_tap_fired) != 0 ? 1 : 0;
    CGEventTapEnable(port, false);
    if (source != NULL) {
        CFRunLoopRemoveSource(CFRunLoopGetCurrent(), source, kCFRunLoopCommonModes);
        CFRelease(source);
    }
    CFMachPortInvalidate(port);
    CFRelease(port);
    return 0;
}

static int g_pixel_control = -1;

static void count_pixels(CGImageRef image, int *nonblack, int *marker) {
    *nonblack = -1;
    *marker = -1;
    g_pixel_control = -1;
    if (image == NULL) {
        return;
    }
    size_t width = CGImageGetWidth(image);
    size_t height = CGImageGetHeight(image);
    if (width == 0 || height == 0 || width > 8192 || height > 8192) {
        return;
    }
    size_t stride = width * 4;
    void *bytes = calloc(height, stride);
    if (bytes == NULL) {
        return;
    }
    CGColorSpaceRef space = CGColorSpaceCreateDeviceRGB();
    CGContextRef context = CGBitmapContextCreate(bytes, width, height, 8, stride, space,
                                                 kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big);
    CGColorSpaceRelease(space);
    if (context == NULL) {
        free(bytes);
        return;
    }
    CGContextDrawImage(context, CGRectMake(0, 0, (CGFloat)width, (CGFloat)height), image);
    CGContextRelease(context);
    int dark = 0;
    int pink = 0;
    int green_window = 0;
    const uint8_t *pixels = bytes;
    size_t count = width * height;
    for (size_t index = 0; index < count; index++) {
        uint8_t red = pixels[index * 4];
        uint8_t green = pixels[index * 4 + 1];
        uint8_t blue = pixels[index * 4 + 2];
        uint8_t alpha = pixels[index * 4 + 3];
        if (alpha == 0) {
            continue;
        }
        if (alpha != 255) {
            red = (uint8_t)((red * 255) / alpha);
            green = (uint8_t)((green * 255) / alpha);
            blue = (uint8_t)((blue * 255) / alpha);
        }
        if (red > 12 || green > 12 || blue > 12) {
            dark++;
        }
        // Probe window is sRGB 0.86, 0.04, 0.63, about 219, 10, 161.
        if (red >= 160 && green <= 80 && blue >= 90 && blue <= 230 && red > green + 40 && blue > green + 40) {
            pink++;
        }
        // Control window is sRGB 0.05, 0.82, 0.08, about 13, 209, 20.
        if (green >= 160 && red <= 80 && blue <= 80 && green > red + 40 && green > blue + 40) {
            green_window++;
        }
    }
    free(bytes);
    *nonblack = dark;
    *marker = pink;
    g_pixel_control = green_window;
}

static void capture_from_image(CGImageRef image, CaptureOut *out, const char *null_detail) {
    if (image == NULL) {
        snprintf(out->detail, sizeof out->detail, "%s", null_detail);
        out->prompt = macos_prompt_code();
        out->nonblack = -1;
        out->marker = -1;
        out->control = -1;
        return;
    }
    out->opened = 1;
    out->width = (int)CGImageGetWidth(image);
    out->height = (int)CGImageGetHeight(image);
    count_pixels(image, &out->nonblack, &out->marker);
    out->control = g_pixel_control;
    CGImageRelease(image);
}

int macos_prove_cg_window_list(CaptureOut *out) {
    if (out == NULL) {
        return 1;
    }
    memset(out, 0, sizeof *out);
    typedef CGImageRef (*CreateImageFn)(CGRect, uint32_t, uint32_t, uint32_t);
    CreateImageFn create = (CreateImageFn)dlsym(RTLD_DEFAULT, "CGWindowListCreateImage");
    if (create == NULL) {
        snprintf(out->detail, sizeof out->detail, "symbol missing");
        return 0;
    }
    CGRect bounds = CGDisplayBounds(CGMainDisplayID());
    CGImageRef image = create(bounds, (uint32_t)kCGWindowListOptionOnScreenOnly,
                              (uint32_t)kCGNullWindowID, (uint32_t)kCGWindowImageDefault);
    capture_from_image(image, out, "returned NULL");
    return 0;
}

int macos_prove_cg_display(CaptureOut *out) {
    if (out == NULL) {
        return 1;
    }
    memset(out, 0, sizeof *out);
    typedef CGImageRef (*CreateDisplayFn)(CGDirectDisplayID);
    CreateDisplayFn create = (CreateDisplayFn)dlsym(RTLD_DEFAULT, "CGDisplayCreateImage");
    if (create == NULL) {
        snprintf(out->detail, sizeof out->detail, "symbol missing; obsoleted in macOS 15");
        return 0;
    }
    CGImageRef image = create(CGMainDisplayID());
    capture_from_image(image, out, "returned NULL");
    return 0;
}

typedef struct {
    int opened;
    int width;
    int height;
    int nonblack;
    int marker;
    int control;
    int error_code;
    char domain[128];
    char detail[384];
} ScResult;

@class StreamSink;
static ScResult g_sc;
static SCStream *g_live_stream = nil;
static StreamSink *g_stream_sink = nil;
static _Atomic int g_sc_done = 0;
static _Atomic int g_sc_gen = 0;

int macos_prove_screen_capture_kit(CaptureOut *out) {
    if (out == NULL) {
        return 1;
    }
    memset(out, 0, sizeof *out);
    if (@available(macOS 12.3, *)) {
        memset(&g_sc, 0, sizeof g_sc);
        atomic_store(&g_sc_done, 0);
        int generation = atomic_fetch_add(&g_sc_gen, 1) + 1;
        [SCShareableContent getShareableContentWithCompletionHandler:^(SCShareableContent *content, NSError *error) {
            if (atomic_load(&g_sc_gen) != generation) {
                return;
            }
            if (error != nil) {
                NSString *domain = error.domain != nil ? error.domain : @"unknown";
                snprintf(g_sc.domain, sizeof g_sc.domain, "%s", domain.UTF8String != NULL ? domain.UTF8String : "");
                g_sc.error_code = (int)error.code;
                atomic_store(&g_sc_done, 1);
                return;
            }
            if (content.displays.count == 0) {
                snprintf(g_sc.detail, sizeof g_sc.detail, "no displays");
                atomic_store(&g_sc_done, 1);
                return;
            }
            SCDisplay *display = content.displays.firstObject;
            if (@available(macOS 14.0, *)) {
                SCContentFilter *filter = [[SCContentFilter alloc] initWithDisplay:display excludingWindows:@[]];
                SCStreamConfiguration *config = [[SCStreamConfiguration alloc] init];
                config.width = (size_t)display.frame.size.width;
                config.height = (size_t)display.frame.size.height;
                [SCScreenshotManager captureImageWithFilter:filter
                                              configuration:config
                                          completionHandler:^(CGImageRef image, NSError *shot_error) {
                    if (atomic_load(&g_sc_gen) != generation) {
                        return;
                    }
                    if (image != NULL) {
                        g_sc.opened = 1;
                        g_sc.width = (int)CGImageGetWidth(image);
                        g_sc.height = (int)CGImageGetHeight(image);
                        count_pixels(image, &g_sc.nonblack, &g_sc.marker);
                        g_sc.control = g_pixel_control;
                    } else if (shot_error != nil) {
                        NSString *domain = shot_error.domain != nil ? shot_error.domain : @"unknown";
                        snprintf(g_sc.domain, sizeof g_sc.domain, "%s",
                                 domain.UTF8String != NULL ? domain.UTF8String : "");
                        g_sc.error_code = (int)shot_error.code;
                        snprintf(g_sc.detail, sizeof g_sc.detail, "SCScreenshotManager");
                    } else {
                        snprintf(g_sc.detail, sizeof g_sc.detail, "SCScreenshotManager returned NULL");
                    }
                    atomic_store(&g_sc_done, 1);
                }];
                return;
            }
            snprintf(g_sc.detail, sizeof g_sc.detail, "SCScreenshotManager unavailable");
            atomic_store(&g_sc_done, 1);
        }];
        NSDate *until = [NSDate dateWithTimeIntervalSinceNow:12];
        while (atomic_load(&g_sc_done) == 0 && [until timeIntervalSinceNow] > 0) {
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.05, true);
        }
        if (atomic_load(&g_sc_done) == 0) {
            snprintf(out->detail, sizeof out->detail, "timed out");
            out->prompt = macos_prompt_code();
            return 0;
        }
        if (g_sc.opened) {
            out->opened = 1;
            out->width = g_sc.width;
            out->height = g_sc.height;
            out->nonblack = g_sc.nonblack;
            out->marker = g_sc.marker;
            out->control = g_sc.control;
            return 0;
        }
        if (g_sc.domain[0] != 0) {
            snprintf(out->detail, sizeof out->detail, "NSError %s %d", g_sc.domain, g_sc.error_code);
        } else if (g_sc.detail[0] != 0) {
            snprintf(out->detail, sizeof out->detail, "%s", g_sc.detail);
        } else {
            snprintf(out->detail, sizeof out->detail, "returned NULL");
        }
        out->prompt = macos_prompt_code();
        return 0;
    }
    snprintf(out->detail, sizeof out->detail, "unavailable");
    return 0;
}

@interface StreamSink : NSObject <SCStreamOutput>
@end

@implementation StreamSink
- (void)stream:(SCStream *)stream didOutputSampleBuffer:(CMSampleBufferRef)sampleBuffer ofType:(SCStreamOutputType)type {
    (void)stream;
    if (type != SCStreamOutputTypeScreen || atomic_load(&g_sc_done) != 0) {
        return;
    }
    CVImageBufferRef buffer = CMSampleBufferGetImageBuffer(sampleBuffer);
    if (buffer == NULL) {
        snprintf(g_sc.detail, sizeof g_sc.detail, "CMSampleBufferGetImageBuffer returned NULL");
        atomic_store(&g_sc_done, 1);
        return;
    }
    CVPixelBufferLockBaseAddress(buffer, kCVPixelBufferLock_ReadOnly);
    size_t width = CVPixelBufferGetWidth(buffer);
    size_t height = CVPixelBufferGetHeight(buffer);
    size_t stride = CVPixelBufferGetBytesPerRow(buffer);
    uint8_t *base = CVPixelBufferGetBaseAddress(buffer);
    OSType format = CVPixelBufferGetPixelFormatType(buffer);
    if (base == NULL || format != kCVPixelFormatType_32BGRA || width == 0 || height == 0 || width > 8192 || height > 8192) {
        snprintf(g_sc.detail, sizeof g_sc.detail, "SCStream pixel format %u", (unsigned)format);
        CVPixelBufferUnlockBaseAddress(buffer, kCVPixelBufferLock_ReadOnly);
        atomic_store(&g_sc_done, 1);
        return;
    }
    int dark = 0;
    int pink = 0;
    int green_window = 0;
    for (size_t y = 0; y < height; y++) {
        uint8_t *row = base + y * stride;
        for (size_t x = 0; x < width; x++) {
            uint8_t blue = row[x * 4];
            uint8_t green = row[x * 4 + 1];
            uint8_t red = row[x * 4 + 2];
            uint8_t alpha = row[x * 4 + 3];
            if (alpha == 0) {
                continue;
            }
            if (red > 12 || green > 12 || blue > 12) {
                dark++;
            }
            if (red >= 160 && green <= 80 && blue >= 90 && blue <= 230 && red > green + 40 && blue > green + 40) {
                pink++;
            }
            if (green >= 160 && red <= 80 && blue <= 80 && green > red + 40 && green > blue + 40) {
                green_window++;
            }
        }
    }
    CVPixelBufferUnlockBaseAddress(buffer, kCVPixelBufferLock_ReadOnly);
    g_sc.opened = 1;
    g_sc.width = (int)width;
    g_sc.height = (int)height;
    g_sc.nonblack = dark;
    g_sc.marker = pink;
    g_sc.control = green_window;
    atomic_store(&g_sc_done, 1);
}
@end

int macos_prove_sc_stream(CaptureOut *out) {
    if (out == NULL) {
        return 1;
    }
    memset(out, 0, sizeof *out);
    out->control = -1;
    if (@available(macOS 12.3, *)) {
        memset(&g_sc, 0, sizeof g_sc);
        g_sc.control = -1;
        atomic_store(&g_sc_done, 0);
        int generation = atomic_fetch_add(&g_sc_gen, 1) + 1;
        [SCShareableContent getShareableContentWithCompletionHandler:^(SCShareableContent *content, NSError *error) {
            if (atomic_load(&g_sc_gen) != generation) {
                return;
            }
            if (error != nil || content.displays.count == 0) {
                if (error != nil) {
                    NSString *domain = error.domain != nil ? error.domain : @"unknown";
                    snprintf(g_sc.domain, sizeof g_sc.domain, "%s", domain.UTF8String != NULL ? domain.UTF8String : "");
                    g_sc.error_code = (int)error.code;
                } else {
                    snprintf(g_sc.detail, sizeof g_sc.detail, "no displays");
                }
                atomic_store(&g_sc_done, 1);
                return;
            }
            SCDisplay *display = content.displays.firstObject;
            SCContentFilter *filter = [[SCContentFilter alloc] initWithDisplay:display excludingWindows:@[]];
            SCStreamConfiguration *config = [[SCStreamConfiguration alloc] init];
            config.width = (size_t)display.frame.size.width;
            config.height = (size_t)display.frame.size.height;
            config.pixelFormat = kCVPixelFormatType_32BGRA;
            config.minimumFrameInterval = CMTimeMake(1, 2);
            config.queueDepth = 1;
            g_stream_sink = [[StreamSink alloc] init];
            g_live_stream = [[SCStream alloc] initWithFilter:filter configuration:config delegate:nil];
            NSError *add_error = nil;
            BOOL added = [g_live_stream addStreamOutput:g_stream_sink type:SCStreamOutputTypeScreen sampleHandlerQueue:dispatch_get_main_queue() error:&add_error];
            if (!added || add_error != nil) {
                NSString *domain = add_error.domain != nil ? add_error.domain : @"SCStream";
                snprintf(g_sc.domain, sizeof g_sc.domain, "%s", domain.UTF8String != NULL ? domain.UTF8String : "");
                g_sc.error_code = add_error != nil ? (int)add_error.code : -1;
                snprintf(g_sc.detail, sizeof g_sc.detail, "addStreamOutput");
                atomic_store(&g_sc_done, 1);
                return;
            }
            [g_live_stream startCaptureWithCompletionHandler:^(NSError *start_error) {
                if (start_error != nil && atomic_load(&g_sc_done) == 0) {
                    NSString *domain = start_error.domain != nil ? start_error.domain : @"SCStream";
                    snprintf(g_sc.domain, sizeof g_sc.domain, "%s", domain.UTF8String != NULL ? domain.UTF8String : "");
                    g_sc.error_code = (int)start_error.code;
                    snprintf(g_sc.detail, sizeof g_sc.detail, "startCapture");
                    atomic_store(&g_sc_done, 1);
                }
            }];
        }];
        NSDate *until = [NSDate dateWithTimeIntervalSinceNow:12];
        while (atomic_load(&g_sc_done) == 0 && [until timeIntervalSinceNow] > 0) {
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.05, true);
        }
        if (g_live_stream != nil) {
            [g_live_stream stopCaptureWithCompletionHandler:^(NSError *stop_error) {
                (void)stop_error;
            }];
            g_live_stream = nil;
            g_stream_sink = nil;
        }
        if (atomic_load(&g_sc_done) == 0) {
            snprintf(out->detail, sizeof out->detail, "SCStream timed out");
            out->prompt = macos_prompt_code();
            return 0;
        }
        if (g_sc.opened) {
            out->opened = 1;
            out->width = g_sc.width;
            out->height = g_sc.height;
            out->nonblack = g_sc.nonblack;
            out->marker = g_sc.marker;
            out->control = g_sc.control;
            return 0;
        }
        if (g_sc.domain[0] != 0) {
            snprintf(out->detail, sizeof out->detail, "NSError %s %d %s", g_sc.domain, g_sc.error_code, g_sc.detail);
        } else if (g_sc.detail[0] != 0) {
            snprintf(out->detail, sizeof out->detail, "%s", g_sc.detail);
        } else {
            snprintf(out->detail, sizeof out->detail, "SCStream returned NULL");
        }
        out->prompt = macos_prompt_code();
        return 0;
    }
    snprintf(out->detail, sizeof out->detail, "unavailable");
    return 0;
}

int macos_session_info(SessionOut *out) {
    if (out == NULL) {
        return 1;
    }
    memset(out, 0, sizeof *out);
    out->uid = (int)getuid();
    out->pid = (int)getpid();
    SecuritySessionId session_id = 0;
    SessionAttributeBits bits = 0;
    OSStatus status = SessionGetInfo(callerSecuritySession, &session_id, &bits);
    out->status = (int)status;
    if (status == 0) {
        out->graphic = (bits & sessionHasGraphicAccess) != 0 ? 1 : 0;
    } else {
        snprintf(out->error, sizeof out->error, "SessionGetInfo failed (%d)", (int)status);
    }
    return 0;
}

static _Atomic int g_active_deleted = 0;
static _Atomic int g_listen_saw = 0;

static CGEventRef active_delete_callback(CGEventTapProxy proxy, CGEventType type, CGEventRef event, void *user) {
    (void)proxy;
    (void)user;
    if (type == kCGEventKeyDown) {
        int64_t key = CGEventGetIntegerValueField(event, kCGKeyboardEventKeycode);
        if (key == kProbeKey) {
            atomic_store(&g_active_deleted, 1);
            return NULL;
        }
    }
    return event;
}

static CGEventRef active_listen_callback(CGEventTapProxy proxy, CGEventType type, CGEventRef event, void *user) {
    (void)proxy;
    (void)user;
    if (type == kCGEventKeyDown) {
        int64_t key = CGEventGetIntegerValueField(event, kCGKeyboardEventKeycode);
        if (key == kProbeKey) {
            atomic_store(&g_listen_saw, 1);
        }
    }
    return event;
}

static void release_tap(CFMachPortRef port, CFRunLoopSourceRef source) {
    if (port != NULL) {
        CGEventTapEnable(port, false);
        CFMachPortInvalidate(port);
    }
    if (source != NULL) {
        CFRunLoopRemoveSource(CFRunLoopGetCurrent(), source, kCFRunLoopCommonModes);
        CFRelease(source);
    }
    if (port != NULL) {
        CFRelease(port);
    }
}

int macos_prove_active_tap(ActiveOut *out) {
    if (out == NULL) {
        return 1;
    }
    memset(out, 0, sizeof *out);
    out->deleted = -1;
    out->listen_saw = -1;
    out->accessibility_trusted = -1;
    out->listen_preflight = -1;
    atomic_store(&g_active_deleted, 0);
    atomic_store(&g_listen_saw, 0);
    out->control_saw = 0;
    fprintf(stderr, "stage=active-control\n");
    fflush(stderr);
    CGEventMask mask = CGEventMaskBit(kCGEventKeyDown);
    CFMachPortRef control = CGEventTapCreate(kCGSessionEventTap, kCGHeadInsertEventTap,
                                             kCGEventTapOptionListenOnly, mask, active_listen_callback, NULL);
    CFRunLoopSourceRef control_source = NULL;
    if (control != NULL) {
        control_source = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, control, 0);
        if (control_source != NULL) {
            CFRunLoopAddSource(CFRunLoopGetCurrent(), control_source, kCFRunLoopCommonModes);
        }
        CGEventTapEnable(control, true);
        CGEventSourceRef control_events = CGEventSourceCreate(kCGEventSourceStateCombinedSessionState);
        if (control_events != NULL) {
            char ignored[64];
            post_key(control_events, kCGSessionEventTap, ignored, sizeof ignored);
            CFRelease(control_events);
        }
        NSDate *control_until = [NSDate dateWithTimeIntervalSinceNow:0.6];
        while (atomic_load(&g_listen_saw) == 0 && [control_until timeIntervalSinceNow] > 0) {
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.05, true);
        }
        out->control_saw = atomic_load(&g_listen_saw) != 0 ? 1 : 0;
    }
    release_tap(control, control_source);
    atomic_store(&g_listen_saw, 0);
    fprintf(stderr, "stage=active-control-done\n");
    fflush(stderr);
    fprintf(stderr, "stage=active-listen\n");
    fflush(stderr);
    CFMachPortRef listen = CGEventTapCreate(kCGSessionEventTap, kCGHeadInsertEventTap,
                                            kCGEventTapOptionListenOnly, mask, active_listen_callback, NULL);
    fprintf(stderr, "stage=active-default\n");
    fflush(stderr);
    CFMachPortRef active = CGEventTapCreate(kCGSessionEventTap, kCGHeadInsertEventTap,
                                            kCGEventTapOptionDefault, mask, active_delete_callback, NULL);
    out->accessibility_trusted = AXIsProcessTrusted() ? 1 : 0;
    preflight_listen(&out->listen_preflight);
    out->prompt = macos_prompt_code();
    CFRunLoopSourceRef listen_source = NULL;
    CFRunLoopSourceRef active_source = NULL;
    if (listen != NULL) {
        listen_source = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, listen, 0);
        if (listen_source != NULL) {
            CFRunLoopAddSource(CFRunLoopGetCurrent(), listen_source, kCFRunLoopCommonModes);
        }
        CGEventTapEnable(listen, true);
    }
    if (active == NULL) {
        out->created = 0;
        snprintf(out->detail, sizeof out->detail, "kCGEventTapOptionDefault returned NULL");
        release_tap(listen, listen_source);
        fprintf(stderr, "stage=active-done\n");
        fflush(stderr);
        return 0;
    }
    out->created = 1;
    active_source = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, active, 0);
    if (active_source != NULL) {
        CFRunLoopAddSource(CFRunLoopGetCurrent(), active_source, kCFRunLoopCommonModes);
    }
    CGEventTapEnable(active, true);
    CGEventSourceRef source = CGEventSourceCreate(kCGEventSourceStateCombinedSessionState);
    if (source == NULL) {
        snprintf(out->detail, sizeof out->detail, "kCGEventTapOptionDefault CGEventSourceCreate returned NULL");
    } else {
        char post_error[256];
        post_error[0] = 0;
        if (post_key(source, kCGHIDEventTap, post_error, sizeof post_error) != 0) {
            snprintf(out->detail, sizeof out->detail, "kCGEventTapOptionDefault %s", post_error);
        }
        NSDate *hid_until = [NSDate dateWithTimeIntervalSinceNow:0.3];
        while (atomic_load(&g_active_deleted) == 0 && [hid_until timeIntervalSinceNow] > 0) {
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.05, true);
        }
        if (atomic_load(&g_active_deleted) == 0 && out->detail[0] == 0) {
            post_error[0] = 0;
            if (post_key(source, kCGSessionEventTap, post_error, sizeof post_error) != 0) {
                snprintf(out->detail, sizeof out->detail, "kCGEventTapOptionDefault %s", post_error);
            }
        }
        CFRelease(source);
    }
    NSDate *until = [NSDate dateWithTimeIntervalSinceNow:0.6];
    while ([until timeIntervalSinceNow] > 0
           && (atomic_load(&g_active_deleted) == 0 || (listen != NULL && atomic_load(&g_listen_saw) == 0))) {
        CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.05, true);
    }
    out->deleted = atomic_load(&g_active_deleted) != 0 ? 1 : 0;
    if (listen == NULL) {
        out->listen_saw = -1;
        snprintf(out->detail, sizeof out->detail,
                 "kCGEventTapOptionDefault ran but the listen-only tap was NULL, so the delete was not compared");
    } else {
        out->listen_saw = atomic_load(&g_listen_saw) != 0 ? 1 : 0;
        if (out->deleted == 1 && out->listen_saw == 1) {
            snprintf(out->detail, sizeof out->detail,
                     "kCGEventTapOptionDefault returned NULL for keyCode 0 but the listen-only tap still observed it");
        } else if (out->deleted == 0 && out->detail[0] == 0) {
            snprintf(out->detail, sizeof out->detail, "kCGEventTapOptionDefault did not delete keyCode 0");
        }
    }
    release_tap(active, active_source);
    release_tap(listen, listen_source);
    fprintf(stderr, "stage=active-done\n");
    fflush(stderr);
    return 0;
}

int macos_inspect_png(const char *path, int *nonblack, int *marker, int *control) {
    if (path == NULL || nonblack == NULL || marker == NULL || control == NULL) {
        return 1;
    }
    *nonblack = -1;
    *marker = -1;
    *control = -1;
    @autoreleasepool {
        NSString *file = [NSString stringWithUTF8String:path];
        NSData *data = file != nil ? [NSData dataWithContentsOfFile:file] : nil;
        NSBitmapImageRep *rep = data != nil ? [NSBitmapImageRep imageRepWithData:data] : nil;
        if (rep == nil || rep.CGImage == NULL) {
            return 1;
        }
        count_pixels(rep.CGImage, nonblack, marker);
        *control = g_pixel_control;
    }
    return (*nonblack < 0 || *marker < 0 || *control < 0) ? 1 : 0;
}

static const int kSlowKey = 1;
static const int kMarkerKey = 105;
static const int kSlowSleepMs = 2000;

typedef struct {
    int64_t state_id;
    int64_t source_pid;
    int64_t poster_pid;
    int window_received;
    int tap_saw;
    int same_process;
    int fields_read;
    char source[64];
    char error[256];
} PostRow;

typedef struct {
    PostRow posts[6];
    int post_count;
    int pad;
    int64_t sleep_ms;
    int64_t disable_wait_ms;
    int64_t dead_ms;
    int64_t healthy_ms;
    int64_t disable_type;
    int key_reached_during;
    int key_reached_after;
    int dead_misses;
    int reenabled;
    int healthy_seen;
    int no_tap;
    char disable_event[64];
    char slow_error[256];
    char marker_error[256];
    char error[512];
} M03Out;

static _Atomic int g_count_post = 0;
static _Atomic int g_count_slow = 0;
static _Atomic int64_t g_last_post_ms = 0;
static _Atomic int64_t g_last_slow_ms = 0;
static _Atomic int64_t g_tap_state = 0;
static _Atomic int64_t g_tap_pid = 0;
static _Atomic int g_tap_saw = 0;
static _Atomic int64_t g_slow_entered_ms = 0;
static _Atomic int64_t g_slow_returned_ms = 0;
static _Atomic int64_t g_disable_type = 0;
static _Atomic int64_t g_disable_ms = 0;
static _Atomic int g_marker_seen = 0;

static int64_t now_ms(void) {
    struct timespec ts;
    clock_gettime(CLOCK_REALTIME, &ts);
    return (int64_t)ts.tv_sec * 1000 + ts.tv_nsec / 1000000;
}

static void write_text(const char *path, const char *text) {
    FILE *file = fopen(path, "w");
    if (file == NULL) {
        return;
    }
    fputs(text, file);
    fclose(file);
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

static void join_path(char *out, size_t cap, const char *dir, const char *name) {
    snprintf(out, cap, "%s/%s", dir, name);
}

static void write_target_state(const char *dir) {
    char body[256];
    snprintf(body, sizeof body, "ready=1\ncount0=%d\ncount1=%d\nlast0=%lld\nlast1=%lld\n",
             atomic_load(&g_count_post), atomic_load(&g_count_slow),
             (long long)atomic_load(&g_last_post_ms), (long long)atomic_load(&g_last_slow_ms));
    char path[1024];
    join_path(path, sizeof path, dir, "target-state.txt");
    write_text(path, body);
}

static int read_int_line(const char *text, const char *key, long long *out) {
    const char *found = strstr(text, key);
    if (found == NULL) {
        return -1;
    }
    return sscanf(found + strlen(key), "%lld", out) == 1 ? 0 : -1;
}

static int read_target_state(const char *dir, int *count0, int *count1, int64_t *last0, int64_t *last1) {
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
    long long c1 = 0;
    long long l0 = 0;
    long long l1 = 0;
    if (read_int_line(text, "ready=", &ready) != 0 || ready != 1) {
        return -1;
    }
    read_int_line(text, "count0=", &c0);
    read_int_line(text, "count1=", &c1);
    read_int_line(text, "last0=", &l0);
    read_int_line(text, "last1=", &l1);
    *count0 = (int)c0;
    *count1 = (int)c1;
    *last0 = (int64_t)l0;
    *last1 = (int64_t)l1;
    return 0;
}

static CGEventRef field_callback(CGEventTapProxy proxy, CGEventType type, CGEventRef event, void *user) {
    (void)proxy;
    (void)user;
    if (type == kCGEventKeyDown && event != NULL) {
        int64_t key = CGEventGetIntegerValueField(event, kCGKeyboardEventKeycode);
        if (key == kProbeKey) {
            atomic_store(&g_tap_state, CGEventGetIntegerValueField(event, kCGEventSourceStateID));
            atomic_store(&g_tap_pid, CGEventGetIntegerValueField(event, kCGEventSourceUnixProcessID));
            atomic_store(&g_tap_saw, 1);
        }
    }
    return event;
}

static CGEventRef slow_callback(CGEventTapProxy proxy, CGEventType type, CGEventRef event, void *user) {
    (void)proxy;
    (void)user;
    if (type == kCGEventTapDisabledByTimeout || type == kCGEventTapDisabledByUserInput) {
        atomic_store(&g_disable_type, (int64_t)type);
        atomic_store(&g_disable_ms, now_ms());
        // A disabled tap is not re-enabled. Returning NULL does not turn it back on.
        return NULL;
    }
    if (type == kCGEventKeyDown && event != NULL) {
        int64_t key = CGEventGetIntegerValueField(event, kCGKeyboardEventKeycode);
        if (key == kSlowKey && atomic_load(&g_slow_entered_ms) == 0) {
            atomic_store(&g_slow_entered_ms, now_ms());
            usleep((useconds_t)kSlowSleepMs * 1000);
            atomic_store(&g_slow_returned_ms, now_ms());
        }
    }
    return event;
}

static CGEventRef marker_callback(CGEventTapProxy proxy, CGEventType type, CGEventRef event, void *user) {
    (void)proxy;
    (void)user;
    if (type == kCGEventTapDisabledByTimeout || type == kCGEventTapDisabledByUserInput) {
        return NULL;
    }
    if (type == kCGEventKeyDown && event != NULL) {
        int64_t key = CGEventGetIntegerValueField(event, kCGKeyboardEventKeycode);
        if (key == kMarkerKey) {
            atomic_store(&g_marker_seen, 1);
        }
    }
    return event;
}

static int post_key_code(CGEventSourceRef source, CGKeyCode key, CGEventTapLocation location, int64_t *state_id,
                         int64_t *source_pid, char *error, size_t cap) {
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
    CGEventPost(location, down);
    CGEventPost(location, up);
    CFRelease(down);
    CFRelease(up);
    return 0;
}

static int post_named(const char *name, const char *dir, CGKeyCode key, char *error, size_t cap) {
    CGEventSourceStateID state;
    if (state_from_name(name, &state) != 0) {
        snprintf(error, cap, "unknown CGEventSourceStateID %s", name);
        return -1;
    }
    CGEventSourceRef source = CGEventSourceCreate(state);
    if (source == NULL) {
        snprintf(error, cap, "CGEventSourceCreate returned NULL");
        return -1;
    }
    int64_t state_id = 0;
    int64_t source_pid = 0;
    int rc = post_key_code(source, key, kCGSessionEventTap, &state_id, &source_pid, error, cap);
    CFRelease(source);
    if (rc != 0) {
        return rc;
    }
    char body[256];
    snprintf(body, sizeof body, "ok=1\nstate_id=%lld\nsource_pid=%lld\nposter_pid=%d\n",
             (long long)state_id, (long long)source_pid, (int)getpid());
    char path[1024];
    join_path(path, sizeof path, dir, "last-post.txt");
    write_text(path, body);
    return 0;
}

int macos_classify_post(const char *source, const char *dir) {
    if (source == NULL || dir == NULL) {
        return 2;
    }
    char error[256];
    error[0] = 0;
    fprintf(stderr, "stage=classify-post\n");
    fflush(stderr);
    if (post_named(source, dir, (CGKeyCode)kProbeKey, error, sizeof error) != 0) {
        char body[512];
        snprintf(body, sizeof body, "ok=0\nstate_id=0\nsource_pid=0\nposter_pid=%d\nerror=%s\n",
                 (int)getpid(), error);
        char path[1024];
        join_path(path, sizeof path, dir, "last-post.txt");
        write_text(path, body);
        fprintf(stderr, "stage=classify-post-done\n");
        return 1;
    }
    fprintf(stderr, "stage=classify-post-done\n");
    return 0;
}

static BOOL open_target(char *error_out, size_t cap) {
    activate_app();
    NSScreen *screen = [NSScreen mainScreen];
    NSRect frame_rect = screen != nil ? screen.visibleFrame : NSMakeRect(0, 0, 800, 600);
    NSRect frame = NSMakeRect(NSMidX(frame_rect) - 160, NSMidY(frame_rect) - 120, 320, 240);
    ProbeWindow *window = [[ProbeWindow alloc] initWithContentRect:frame
                                                          styleMask:NSWindowStyleMaskBorderless
                                                            backing:NSBackingStoreBuffered
                                                              defer:NO];
    if (window == nil) {
        snprintf(error_out, cap, "NSWindow initWithContentRect returned nil");
        return NO;
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
    return wait_until_key(window);
}

int macos_classify_target(const char *dir) {
    if (dir == NULL) {
        return 2;
    }
    fprintf(stderr, "stage=classify-target\n");
    fflush(stderr);
    @autoreleasepool {
        char ignored[64];
        if (!open_target(ignored, sizeof ignored)) {
            fprintf(stderr, "stage=classify-target-not-key\n");
        }
        id monitor = [NSEvent addLocalMonitorForEventsMatchingMask:NSEventMaskKeyDown
                                                            handler:^NSEvent *(NSEvent *event) {
                                                              int code = (int)event.keyCode;
                                                              int64_t stamp = now_ms();
                                                              if (code == kProbeKey) {
                                                                  atomic_fetch_add(&g_count_post, 1);
                                                                  atomic_store(&g_last_post_ms, stamp);
                                                              }
                                                              if (code == kSlowKey) {
                                                                  atomic_fetch_add(&g_count_slow, 1);
                                                                  atomic_store(&g_last_slow_ms, stamp);
                                                              }
                                                              return event;
                                                            }];
        char stop[1024];
        join_path(stop, sizeof stop, dir, "target.stop");
        NSDate *until = [NSDate dateWithTimeIntervalSinceNow:40];
        while ([until timeIntervalSinceNow] > 0) {
            write_target_state(dir);
            if (access(stop, F_OK) == 0) {
                break;
            }
            pump(0.05);
        }
        if (monitor != nil) {
            [NSEvent removeMonitor:monitor];
        }
        if (g_window != nil) {
            g_window.releasedWhenClosed = NO;
            [g_window orderOut:nil];
            g_window = nil;
        }
    }
    fprintf(stderr, "stage=classify-target-done\n");
    return 0;
}

static int spawn_child(const char *exe, char *const argv[], pid_t *pid) {
    int rc = posix_spawn(pid, exe, NULL, NULL, argv, environ);
    return rc;
}

static void fill_post(PostRow *row, const char *source, int same_process, int tap_saw, int64_t tap_state,
                      int64_t tap_pid, int before, int after, const char *dir) {
    memset(row, 0, sizeof *row);
    snprintf(row->source, sizeof row->source, "%s", source);
    row->same_process = same_process;
    row->tap_saw = tap_saw;
    row->window_received = after > before ? 1 : 0;
    char path[1024];
    join_path(path, sizeof path, dir, "last-post.txt");
    FILE *file = fopen(path, "r");
    char text[512];
    text[0] = 0;
    if (file != NULL) {
        size_t n = fread(text, 1, sizeof text - 1, file);
        fclose(file);
        text[n] = 0;
    }
    long long ok = 0;
    long long state_id = 0;
    long long source_pid = 0;
    long long poster_pid = 0;
    read_int_line(text, "ok=", &ok);
    read_int_line(text, "state_id=", &state_id);
    read_int_line(text, "source_pid=", &source_pid);
    read_int_line(text, "poster_pid=", &poster_pid);
    if (tap_saw) {
        row->state_id = tap_state;
        row->source_pid = tap_pid;
        row->fields_read = 1;
    } else if (ok == 1) {
        row->state_id = (int64_t)state_id;
        row->source_pid = (int64_t)source_pid;
        row->fields_read = 1;
        snprintf(row->error, sizeof row->error, "tap did not see the key; fields are from the poster");
    } else {
        const char *err = strstr(text, "error=");
        snprintf(row->error, sizeof row->error, "%s", err != NULL ? err + 6 : "CGEventPost wrote no fields");
        char *newline = strchr(row->error, '\n');
        if (newline != NULL) {
            *newline = 0;
        }
    }
    row->poster_pid = (int64_t)poster_pid;
}

static void one_post(const char *exe, const char *dir, const char *source, int same_process, M03Out *out) {
    if (out->post_count >= 6) {
        return;
    }
    /* A single second-process post is sometimes missed by the tap. Repeat it, and keep the attempt the tap saw. */
    int attempts = same_process ? 1 : 5;
    for (int attempt = 0; attempt < attempts; attempt++) {
        atomic_store(&g_tap_saw, 0);
        int before = 0;
        int ignored = 0;
        int64_t last0 = 0;
        int64_t last1 = 0;
        read_target_state(dir, &before, &ignored, &last0, &last1);
        char last[1024];
        join_path(last, sizeof last, dir, "last-post.txt");
        unlink(last);
        char error[256];
        error[0] = 0;
        if (same_process) {
            post_named(source, dir, (CGKeyCode)kProbeKey, error, sizeof error);
        } else {
            char *argv[] = {(char *)exe, "--classify-post", (char *)source, "--output-dir", (char *)dir, NULL};
            pid_t pid = 0;
            if (spawn_child(exe, argv, &pid) != 0) {
                snprintf(error, sizeof error, "posix_spawn of CGEventPost failed");
            } else {
                int status = 0;
                waitpid(pid, &status, 0);
            }
        }
        NSDate *until = [NSDate dateWithTimeIntervalSinceNow:0.6];
        while (atomic_load(&g_tap_saw) == 0 && [until timeIntervalSinceNow] > 0) {
            run_loop(0.05);
        }
        run_loop(0.15);
        int after = before;
        read_target_state(dir, &after, &ignored, &last0, &last1);
        PostRow *row = &out->posts[out->post_count];
        fill_post(row, source, same_process, atomic_load(&g_tap_saw), atomic_load(&g_tap_state), atomic_load(&g_tap_pid),
                  before, after, dir);
        if (row->error[0] == 0 && error[0] != 0) {
            snprintf(row->error, sizeof row->error, "%s", error);
        }
        if (same_process || row->tap_saw) {
            break;
        }
    }
    out->post_count += 1;
}

struct SlowPoster {
    char dir[1024];
};

static void *slow_poster(void *arg) {
    struct SlowPoster *poster = arg;
    NSDate *until = [NSDate dateWithTimeIntervalSinceNow:5];
    while (atomic_load(&g_slow_entered_ms) == 0 && [until timeIntervalSinceNow] > 0) {
        usleep(5000);
    }
    usleep(200000);
    char error[256];
    post_named("kCGEventSourceStateCombinedSessionState", poster->dir, (CGKeyCode)kSlowKey, error, sizeof error);
    return NULL;
}

static void *slow_starter(void *arg) {
    struct SlowPoster *poster = arg;
    usleep(100000);
    char error[256];
    post_named("kCGEventSourceStateCombinedSessionState", poster->dir, (CGKeyCode)kSlowKey, error, sizeof error);
    return NULL;
}

static const char *disable_name(int64_t type) {
    uint32_t value = (uint32_t)type;
    if (value == 0xFFFFFFFE) {
        return "kCGEventTapDisabledByTimeout";
    }
    if (value == 0xFFFFFFFF) {
        return "kCGEventTapDisabledByUserInput";
    }
    return "none";
}

static void run_slow(const char *dir, M03Out *out) {
    fprintf(stderr, "stage=slow-callback\n");
    fflush(stderr);
    atomic_store(&g_slow_entered_ms, 0);
    atomic_store(&g_slow_returned_ms, 0);
    atomic_store(&g_disable_type, 0);
    atomic_store(&g_disable_ms, 0);
    CGEventMask mask = CGEventMaskBit(kCGEventKeyDown);
    CFMachPortRef port = CGEventTapCreate(kCGSessionEventTap, kCGHeadInsertEventTap, kCGEventTapOptionDefault, mask,
                                          slow_callback, NULL);
    if (port == NULL) {
        out->no_tap = 1;
        snprintf(out->slow_error, sizeof out->slow_error, "CGEventTapCreate returned NULL");
        snprintf(out->disable_event, sizeof out->disable_event, "none");
        return;
    }
    CFRunLoopSourceRef source = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, port, 0);
    CFRunLoopAddSource(CFRunLoopGetMain(), source, kCFRunLoopCommonModes);
    CGEventTapEnable(port, true);
    int before = 0;
    int count0 = 0;
    int64_t last0 = 0;
    int64_t last1 = 0;
    read_target_state(dir, &count0, &before, &last0, &last1);
    struct SlowPoster poster;
    snprintf(poster.dir, sizeof poster.dir, "%s", dir);
    pthread_t starter;
    pthread_t during;
    pthread_create(&starter, NULL, slow_starter, &poster);
    pthread_create(&during, NULL, slow_poster, &poster);
    NSDate *until = [NSDate dateWithTimeIntervalSinceNow:8];
    int64_t after_return_budget = 0;
    while ([until timeIntervalSinceNow] > 0) {
        int64_t returned = atomic_load(&g_slow_returned_ms);
        if (returned != 0 && after_return_budget == 0) {
            after_return_budget = now_ms() + 1500;
        }
        run_loop(0.05);
        if (returned != 0 && (atomic_load(&g_disable_type) != 0 || now_ms() >= after_return_budget)) {
            break;
        }
    }
    pthread_join(starter, NULL);
    pthread_join(during, NULL);
    int after = before;
    read_target_state(dir, &count0, &after, &last0, &last1);
    int64_t entered = atomic_load(&g_slow_entered_ms);
    int64_t returned = atomic_load(&g_slow_returned_ms);
    int64_t disable_type = atomic_load(&g_disable_type);
    out->disable_type = disable_type;
    snprintf(out->disable_event, sizeof out->disable_event, "%s", disable_name(disable_type));
    out->sleep_ms = (entered != 0 && returned != 0) ? (returned - entered) : 0;
    if (disable_type != 0 && entered != 0) {
        out->disable_wait_ms = atomic_load(&g_disable_ms) - entered;
    } else if (entered != 0) {
        out->disable_wait_ms = now_ms() - entered;
    }
    out->key_reached_during =
        (after > before && last1 >= entered && returned != 0 && last1 <= returned) ? 1 : 0;
    out->key_reached_after = (after > before && returned != 0 && last1 > returned) ? 1 : 0;
    if (entered == 0) {
        snprintf(out->slow_error, sizeof out->slow_error, "CGEventPost slow key never entered the callback");
    }
    CGEventTapEnable(port, false);
    CFRunLoopRemoveSource(CFRunLoopGetMain(), source, kCFRunLoopCommonModes);
    CFRelease(source);
    CFRelease(port);
    fprintf(stderr, "stage=slow-callback-done\n");
    fflush(stderr);
}

static int post_marker(char *error, size_t cap) {
    CGEventSourceRef source = CGEventSourceCreate(kCGEventSourceStatePrivate);
    if (source == NULL) {
        snprintf(error, cap, "CGEventSourceCreate returned NULL");
        return -1;
    }
    int64_t state_id = 0;
    int64_t source_pid = 0;
    int rc = post_key_code(source, (CGKeyCode)kMarkerKey, kCGSessionEventTap, &state_id, &source_pid, error, cap);
    CFRelease(source);
    return rc;
}

static void run_marker(M03Out *out) {
    fprintf(stderr, "stage=marker\n");
    fflush(stderr);
    atomic_store(&g_marker_seen, 0);
    CGEventMask mask = CGEventMaskBit(kCGEventKeyDown);
    CFMachPortRef port = CGEventTapCreate(kCGSessionEventTap, kCGHeadInsertEventTap, kCGEventTapOptionListenOnly, mask,
                                          marker_callback, NULL);
    if (port == NULL) {
        out->no_tap = 1;
        snprintf(out->marker_error, sizeof out->marker_error, "CGEventTapCreate returned NULL");
        return;
    }
    // The source stays off the run loop, so this callback never runs.
    int64_t start = now_ms();
    int misses = 0;
    while (misses < 3) {
        char error[256];
        error[0] = 0;
        if (post_marker(error, sizeof error) != 0 && out->marker_error[0] == 0) {
            snprintf(out->marker_error, sizeof out->marker_error, "%s", error);
        }
        usleep(250000);
        if (atomic_load(&g_marker_seen) != 0) {
            break;
        }
        misses += 1;
    }
    out->dead_ms = now_ms() - start;
    out->dead_misses = misses;
    atomic_store(&g_marker_seen, 0);
    CFRunLoopSourceRef source = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, port, 0);
    CFRunLoopAddSource(CFRunLoopGetMain(), source, kCFRunLoopCommonModes);
    CGEventTapEnable(port, true);
    int64_t healthy_start = now_ms();
    char error[256];
    error[0] = 0;
    if (post_marker(error, sizeof error) != 0 && out->marker_error[0] == 0) {
        snprintf(out->marker_error, sizeof out->marker_error, "%s", error);
    }
    NSDate *until = [NSDate dateWithTimeIntervalSinceNow:0.25];
    while (atomic_load(&g_marker_seen) == 0 && [until timeIntervalSinceNow] > 0) {
        run_loop(0.01);
    }
    out->healthy_ms = now_ms() - healthy_start;
    out->healthy_seen = atomic_load(&g_marker_seen) != 0 ? 1 : 0;
    if (atomic_load(&g_marker_seen) == 0 && out->marker_error[0] == 0) {
        snprintf(out->marker_error, sizeof out->marker_error,
                 "marker callback did not see the no-op within one heartbeat");
    }
    CGEventTapEnable(port, false);
    CFRunLoopRemoveSource(CFRunLoopGetMain(), source, kCFRunLoopCommonModes);
    CFRelease(source);
    CFRelease(port);
    fprintf(stderr, "stage=marker-done\n");
    fflush(stderr);
}

int macos_prove_m03(const char *exe, const char *dir, M03Out *out) {
    if (exe == NULL || dir == NULL || out == NULL) {
        return 2;
    }
    memset(out, 0, sizeof *out);
    snprintf(out->disable_event, sizeof out->disable_event, "none");
    out->reenabled = 0;
    char stop[1024];
    join_path(stop, sizeof stop, dir, "target.stop");
    unlink(stop);
    char *argv[] = {(char *)exe, "--classify-target", "--output-dir", (char *)dir, NULL};
    pid_t target = 0;
    if (spawn_child(exe, argv, &target) != 0) {
        snprintf(out->error, sizeof out->error, "posix_spawn of the target window failed");
        return 0;
    }
    int ready = 0;
    NSDate *until = [NSDate dateWithTimeIntervalSinceNow:5];
    while ([until timeIntervalSinceNow] > 0) {
        int c0 = 0;
        int c1 = 0;
        int64_t l0 = 0;
        int64_t l1 = 0;
        if (read_target_state(dir, &c0, &c1, &l0, &l1) == 0) {
            ready = 1;
            break;
        }
        pump(0.05);
    }
    if (!ready) {
        snprintf(out->error, sizeof out->error, "NSWindow target did not become ready");
    }
    CGEventMask mask = CGEventMaskBit(kCGEventKeyDown);
    CFMachPortRef listen = CGEventTapCreate(kCGSessionEventTap, kCGHeadInsertEventTap, kCGEventTapOptionListenOnly,
                                            mask, field_callback, NULL);
    CFRunLoopSourceRef listen_source = NULL;
    if (listen == NULL) {
        out->no_tap = 1;
        snprintf(out->error, sizeof out->error, "CGEventTapCreate returned NULL");
    } else {
        listen_source = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, listen, 0);
        CFRunLoopAddSource(CFRunLoopGetMain(), listen_source, kCFRunLoopCommonModes);
        CGEventTapEnable(listen, true);
        const char *sources[3] = {
            "kCGEventSourceStatePrivate",
            "kCGEventSourceStateCombinedSessionState",
            "kCGEventSourceStateHIDSystemState",
        };
        for (int index = 0; index < 3; index++) {
            one_post(exe, dir, sources[index], 0, out);
            one_post(exe, dir, sources[index], 1, out);
        }
        CGEventTapEnable(listen, false);
        CFRunLoopRemoveSource(CFRunLoopGetMain(), listen_source, kCFRunLoopCommonModes);
        CFRelease(listen_source);
        CFRelease(listen);
    }
    run_slow(dir, out);
    run_marker(out);
    write_text(stop, "stop\n");
    kill(target, SIGTERM);
    waitpid(target, NULL, 0);
    return 0;
}
