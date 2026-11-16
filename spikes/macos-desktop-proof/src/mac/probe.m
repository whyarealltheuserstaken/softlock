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
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

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

static void stage_line(WindowOut *out, const char *name) {
    if (out != NULL) {
        snprintf(out->stage, sizeof out->stage, "%s", name);
    }
    fprintf(stderr, "stage=%s\n", name);
    fflush(stderr);
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

static void count_pixels(CGImageRef image, int *nonblack, int *marker) {
    *nonblack = -1;
    *marker = -1;
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
    }
    free(bytes);
    *nonblack = dark;
    *marker = pink;
}

static void capture_from_image(CGImageRef image, CaptureOut *out, const char *null_detail) {
    if (image == NULL) {
        snprintf(out->detail, sizeof out->detail, "%s", null_detail);
        out->prompt = macos_prompt_code();
        out->nonblack = -1;
        out->marker = -1;
        return;
    }
    out->opened = 1;
    out->width = (int)CGImageGetWidth(image);
    out->height = (int)CGImageGetHeight(image);
    count_pixels(image, &out->nonblack, &out->marker);
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

int macos_inspect_png(const char *path, int *nonblack, int *marker) {
    if (path == NULL || nonblack == NULL || marker == NULL) {
        return 1;
    }
    *nonblack = -1;
    *marker = -1;
    @autoreleasepool {
        NSString *file = [NSString stringWithUTF8String:path];
        NSData *data = file != nil ? [NSData dataWithContentsOfFile:file] : nil;
        NSBitmapImageRep *rep = data != nil ? [NSBitmapImageRep imageRepWithData:data] : nil;
        if (rep == nil || rep.CGImage == NULL) {
            return 1;
        }
        count_pixels(rep.CGImage, nonblack, marker);
    }
    return (*nonblack < 0 || *marker < 0) ? 1 : 0;
}
