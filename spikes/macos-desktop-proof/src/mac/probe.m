// Throwaway desktop probe. Listen-only taps return the event unchanged.
// This file does not lock the session and does not read hardware identifiers.

#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>
#import <CoreGraphics/CoreGraphics.h>
#import <Foundation/Foundation.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>

#include <Security/AuthSession.h>
#include <ctype.h>
#include <dlfcn.h>
#include <stdatomic.h>
#include <stdbool.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

typedef struct {
    int ordered_front;
    int key_window;
    int key_received;
    char post_location[64];
    char error[768];
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
    int prompt;
    char detail[768];
} CaptureOut;

typedef struct {
    int uid;
    int pid;
    int graphic;
    int status;
    char error[256];
} SessionOut;

static const int kProbeKey = 0;

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
    if (@available(macOS 14.0, *)) {
        [app activate];
    } else {
        [app activateIgnoringOtherApps:YES];
    }
}

int macos_prove_window(WindowOut *out) {
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
            window.opaque = YES;
            window.hasShadow = NO;
            window.level = NSNormalWindowLevel;
            window.backgroundColor = [NSColor colorWithSRGBRed:0.86 green:0.04 blue:0.63 alpha:1];
            ProbeView *view = [[ProbeView alloc] initWithFrame:NSMakeRect(0, 0, 320, 240)];
            window.contentView = view;
            [window orderFrontRegardless];
            [window makeKeyAndOrderFront:nil];
            pump(0.3);

            BOOL on_screen = window.isVisible && window.screen != nil;
            BOOL unoccluded = (window.occlusionState & NSWindowOcclusionStateVisible) != 0;
            out->ordered_front = (on_screen && unoccluded) ? 1 : 0;
            out->key_window = window.isKeyWindow ? 1 : 0;
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
                if (g_key_received == 0 && out->error[0] == 0) {
                    snprintf(out->error, sizeof out->error,
                             "CGEventPost did not deliver keyCode %d (tried kCGSessionEventTap, "
                             "kCGAnnotatedSessionEventTap, kCGHIDEventTap); CGPreflightPostEventAccess=%s",
                             kProbeKey, post_preflight());
                }
                CFRelease(source);
            }
            out->key_received = g_key_received != 0 ? 1 : 0;
            if (monitor != nil) {
                [NSEvent removeMonitor:monitor];
            }
            [window orderOut:nil];
            [window close];
        } @catch (NSException *exception) {
            const char *name = exception.name.UTF8String != NULL ? exception.name.UTF8String : "NSException";
            snprintf(out->error, sizeof out->error, "NSApplication exception %s", name);
        }
    }
    return 0;
}

static _Atomic int g_tap_fired = 0;

static CGEventRef tap_callback(CGEventTapProxy proxy, CGEventType type, CGEventRef event, void *user) {
    (void)proxy;
    (void)type;
    (void)user;
    atomic_store(&g_tap_fired, 1);
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

static void capture_from_image(CGImageRef image, CaptureOut *out, const char *null_detail) {
    if (image == NULL) {
        snprintf(out->detail, sizeof out->detail, "%s", null_detail);
        out->prompt = macos_prompt_code();
        return;
    }
    out->opened = 1;
    out->width = (int)CGImageGetWidth(image);
    out->height = (int)CGImageGetHeight(image);
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
    int error_code;
    char domain[128];
    char detail[384];
} ScResult;

static ScResult g_sc;
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
            } else if (content.displays.count > 0) {
                SCDisplay *display = content.displays.firstObject;
                g_sc.opened = 1;
                g_sc.width = (int)display.frame.size.width;
                g_sc.height = (int)display.frame.size.height;
            } else {
                snprintf(g_sc.detail, sizeof g_sc.detail, "no displays");
            }
            atomic_store(&g_sc_done, 1);
        }];
        NSDate *until = [NSDate dateWithTimeIntervalSinceNow:8];
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
