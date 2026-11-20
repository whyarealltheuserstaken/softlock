// Throwaway m06 overlay kit. Does not lock the session.
// Does not read hardware identifiers. Overlay ignores the mouse and is not key.
// Escape or process teardown closes every covering window.

#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>
#import <CoreGraphics/CoreGraphics.h>
#import <Foundation/Foundation.h>
#import <CoreMedia/CoreMedia.h>
#import <CoreVideo/CoreVideo.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>

#include <ctype.h>
#include <dlfcn.h>
#include <stdatomic.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/sysctl.h>
#include <unistd.h>

typedef struct {
    int ordered_front;
    int is_key;
    int is_main;
    int ignores_mouse_events;
    int shielding_called;
    int display_count;
    int covered_count;
    int window_area;
    int visible_after_none;
    char level[64];
    char collection[128];
    char sharing[64];
    char display_image_api[128];
    char error[256];
} OverlayOut;

typedef struct {
    int opened;
    int width;
    int height;
    int marker;
    int control;
    char detail[384];
} CaptureOut;

typedef struct {
    int overlay_is_key;
    int target_is_key;
    int click_count;
    int key_count;
    int accessibility;
    int listen;
    char error[256];
} ClickOut;

typedef struct {
    int app_active;
    int process_switching;
    int force_quit;
    int session_termination;
    char error[256];
} PresentOut;

static NSMutableArray<NSWindow *> *g_overlay = nil;
static NSWindow *g_control = nil;
static NSWindow *g_target = nil;
static NSButton *g_target_button = nil;
static atomic_int g_escape = 0;
static atomic_int g_clicks = 0;
static atomic_int g_keys = 0;
static id g_escape_monitor = nil;
static id g_local_escape = nil;
static id g_screen_observer = nil;
static int g_pixel_control = -1;

static NSString *g_banner = nil;

static void pump(NSTimeInterval seconds) {
    NSEvent *pending = [NSApp nextEventMatchingMask:NSEventMaskAny
                                          untilDate:[NSDate distantPast]
                                             inMode:NSDefaultRunLoopMode
                                            dequeue:YES];
    while (pending) {
        [NSApp sendEvent:pending];
        pending = [NSApp nextEventMatchingMask:NSEventMaskAny
                                     untilDate:[NSDate distantPast]
                                        inMode:NSDefaultRunLoopMode
                                       dequeue:YES];
    }
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

static void activate_app(void) {
    NSApplication *app = [NSApplication sharedApplication];
    [app setActivationPolicy:NSApplicationActivationPolicyRegular];
    [app activateIgnoringOtherApps:YES];
}

static BOOL wait_until(NSTimeInterval seconds, int (^pred)(void)) {
    NSDate *until = [NSDate dateWithTimeIntervalSinceNow:seconds];
    while ([until timeIntervalSinceNow] > 0) {
        if (pred()) {
            return YES;
        }
        pump(0.02);
    }
    return pred() ? YES : NO;
}

static CGPoint quartz_point_in_view(NSView *view) {
    NSRect bounds = view.bounds;
    NSRect window_rect = [view convertRect:bounds toView:nil];
    NSWindow *window = view.window;
    NSRect screen_rect = [window convertRectToScreen:window_rect];
    NSScreen *screen = window.screen;
    if (screen == nil) {
        screen = [NSScreen mainScreen];
    }
    NSRect frame = screen != nil ? screen.frame : NSMakeRect(0, 0, 1024, 768);
    return CGPointMake(NSMidX(screen_rect), NSMaxY(frame) - NSMidY(screen_rect));
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

@interface TargetWindow : NSWindow
@end
@implementation TargetWindow
- (BOOL)canBecomeKeyWindow {
    return YES;
}
- (BOOL)canBecomeMainWindow {
    return YES;
}
@end

@interface TargetView : NSView
@end
@implementation TargetView
- (BOOL)acceptsFirstResponder {
    return YES;
}
- (BOOL)acceptsFirstMouse:(NSEvent *)event {
    (void)event;
    return YES;
}
- (void)mouseDown:(NSEvent *)event {
    (void)event;
    atomic_fetch_add(&g_clicks, 1);
}
- (void)keyDown:(NSEvent *)event {
    (void)event;
    atomic_fetch_add(&g_keys, 1);
}
@end

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

static void fill_display_image_api(char *buf, size_t len) {
    buf[0] = 0;
    if (dlsym(RTLD_DEFAULT, "CGWindowListCreateImage") != NULL) {
        strncat(buf, "CGWindowListCreateImage", len - 1);
    }
    if (dlsym(RTLD_DEFAULT, "CGDisplayCreateImage") != NULL) {
        if (buf[0] != 0) {
            strncat(buf, ",", len - strlen(buf) - 1);
        }
        strncat(buf, "CGDisplayCreateImage", len - strlen(buf) - 1);
    }
}

static void install_escape_monitor(void) {
    atomic_store(&g_escape, 0);
    if (g_escape_monitor != nil) {
        [NSEvent removeMonitor:g_escape_monitor];
        g_escape_monitor = nil;
    }
    if (g_local_escape != nil) {
        [NSEvent removeMonitor:g_local_escape];
        g_local_escape = nil;
    }
    void (^handler)(NSEvent *) = ^(NSEvent *event) {
      if (event.keyCode == 53) {
          atomic_store(&g_escape, 1);
      }
    };
    g_escape_monitor = [NSEvent addGlobalMonitorForEventsMatchingMask:NSEventMaskKeyDown handler:handler];
    g_local_escape = [NSEvent addLocalMonitorForEventsMatchingMask:NSEventMaskKeyDown
                                                           handler:^NSEvent *(NSEvent *event) {
                                                             if (event.keyCode == 53) {
                                                                 atomic_store(&g_escape, 1);
                                                             }
                                                             return event;
                                                           }];
}

static void apply_banner(NSWindow *window) {
    NSView *content = window.contentView;
    if (content == nil) {
        return;
    }
    NSTextField *label = [content viewWithTag:42];
    NSString *text = g_banner != nil ? g_banner : @"";
    if (label == nil) {
        label = [NSTextField wrappingLabelWithString:text];
        label.tag = 42;
        label.textColor = [NSColor whiteColor];
        label.backgroundColor = [NSColor colorWithWhite:0 alpha:0.4];
        label.drawsBackground = YES;
        label.font = [NSFont boldSystemFontOfSize:28];
        label.frame = NSInsetRect(content.bounds, 48, 96);
        label.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
        [content addSubview:label];
    } else {
        label.stringValue = text;
    }
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
    window.level = NSScreenSaverWindowLevel;
    window.backgroundColor = [NSColor colorWithSRGBRed:0.86 green:0.04 blue:0.63 alpha:1];
    window.collectionBehavior = NSWindowCollectionBehaviorCanJoinAllSpaces |
                                NSWindowCollectionBehaviorFullScreenAuxiliary;
    [window orderFrontRegardless];
    apply_banner(window);
    return window;
}

static BOOL overlays_visible(void) {
    if (g_overlay.count == 0) {
        return NO;
    }
    for (NSWindow *window in g_overlay) {
        if (!window.isVisible || (window.occlusionState & NSWindowOcclusionStateVisible) == 0) {
            return NO;
        }
    }
    return YES;
}

static BOOL overlays_hidden(void) {
    if (g_overlay.count == 0) {
        return YES;
    }
    for (NSWindow *window in g_overlay) {
        if (window.isVisible && (window.occlusionState & NSWindowOcclusionStateVisible) != 0) {
            return NO;
        }
    }
    return YES;
}

int m06_overlay_wait_visible(uint32_t timeout_ms) {
    @autoreleasepool {
        NSDate *until = [NSDate dateWithTimeIntervalSinceNow:timeout_ms / 1000.0];
        while ([until timeIntervalSinceNow] > 0) {
            if (overlays_visible()) {
                return 1;
            }
            pump(0.05);
        }
        return overlays_visible() ? 1 : 0;
    }
}

int m06_overlay_wait_hidden(uint32_t timeout_ms) {
    @autoreleasepool {
        NSDate *until = [NSDate dateWithTimeIntervalSinceNow:timeout_ms / 1000.0];
        while ([until timeIntervalSinceNow] > 0) {
            if (overlays_hidden()) {
                return 1;
            }
            pump(0.05);
        }
        return overlays_hidden() ? 1 : 0;
    }
}

int m06_overlay_set_banner(const char *utf8) {
    @autoreleasepool {
        g_banner = utf8 != NULL ? [NSString stringWithUTF8String:utf8] : @"";
        for (NSWindow *window in g_overlay) {
            apply_banner(window);
        }
    }
    return 0;
}

int m06_screen_capture_access(void) {
    return CGPreflightScreenCaptureAccess() ? 1 : 0;
}

static void fill_overlay_out(OverlayOut *out) {
    out->display_count = (int)[NSScreen screens].count;
    out->covered_count = (int)g_overlay.count;
    out->shielding_called = 0;
    BOOL front = g_overlay.count > 0 && g_overlay.count == [NSScreen screens].count;
    int area = 0;
    BOOL key = NO;
    BOOL main = NO;
    BOOL ignores = YES;
    NSScreen *mainScreen = [NSScreen mainScreen];
    for (NSWindow *window in g_overlay) {
        if (!window.isVisible) {
            front = NO;
        }
        if (window.isKeyWindow) {
            key = YES;
        }
        if (window.isMainWindow) {
            main = YES;
        }
        if (!window.ignoresMouseEvents) {
            ignores = NO;
        }
        if (window.screen == mainScreen || (mainScreen == nil && area == 0)) {
            area = (int)(NSWidth(window.frame) * NSHeight(window.frame));
        }
        if (window.level == NSScreenSaverWindowLevel) {
            snprintf(out->level, sizeof out->level, "screenSaver");
        } else {
            snprintf(out->level, sizeof out->level, "level-%d", (int)window.level);
        }
        sharing_collection(window, out->collection, sizeof out->collection);
        snprintf(out->sharing, sizeof out->sharing, "%s", sharing_name(window.sharingType));
        out->visible_after_none = (window.isVisible &&
                                   (window.occlusionState & NSWindowOcclusionStateVisible) != 0)
                                      ? 1
                                      : 0;
    }
    out->ordered_front = front ? 1 : 0;
    out->is_key = key ? 1 : 0;
    out->is_main = main ? 1 : 0;
    out->ignores_mouse_events = ignores && g_overlay.count > 0 ? 1 : 0;
    out->window_area = area;
    fill_display_image_api(out->display_image_api, sizeof out->display_image_api);
    if (!front) {
        snprintf(out->error, sizeof out->error, "overlay did not cover every NSScreen");
    }
}

static void rebuild_covers(void) {
    for (NSWindow *window in g_overlay) {
        [window orderOut:nil];
        [window close];
    }
    g_overlay = [NSMutableArray array];
    for (NSScreen *screen in [NSScreen screens]) {
        [g_overlay addObject:make_cover(screen.frame)];
    }
}

int m06_overlay_open(OverlayOut *out) {
    if (out == NULL) {
        return 1;
    }
    memset(out, 0, sizeof *out);
    @autoreleasepool {
        ensure_app();
        rebuild_covers();
        install_escape_monitor();
        if (g_screen_observer == nil) {
            g_screen_observer = [[NSNotificationCenter defaultCenter]
                addObserverForName:NSApplicationDidChangeScreenParametersNotification
                            object:nil
                             queue:nil
                        usingBlock:^(NSNotification *note) {
                          (void)note;
                          rebuild_covers();
                        }];
        }
        m06_overlay_wait_visible(2000);
        fill_overlay_out(out);
    }
    return 0;
}

int m06_overlay_set_none(OverlayOut *out) {
    if (out == NULL) {
        return 1;
    }
    memset(out, 0, sizeof *out);
    @autoreleasepool {
        for (NSWindow *window in g_overlay) {
            window.sharingType = NSWindowSharingNone;
        }
        m06_overlay_wait_visible(2000);
        fill_overlay_out(out);
    }
    return 0;
}

int m06_overlay_order_out(void) {
    @autoreleasepool {
        for (NSWindow *window in g_overlay) {
            [window orderOut:nil];
        }
        if (g_control != nil) {
            [g_control orderOut:nil];
        }
        m06_overlay_wait_hidden(2000);
    }
    return 0;
}

int m06_overlay_close(void) {
    @autoreleasepool {
        if (g_escape_monitor != nil) {
            [NSEvent removeMonitor:g_escape_monitor];
            g_escape_monitor = nil;
        }
        if (g_local_escape != nil) {
            [NSEvent removeMonitor:g_local_escape];
            g_local_escape = nil;
        }
        if (g_screen_observer != nil) {
            [[NSNotificationCenter defaultCenter] removeObserver:g_screen_observer];
            g_screen_observer = nil;
        }
        for (NSWindow *window in g_overlay) {
            [window orderOut:nil];
            [window close];
        }
        g_overlay = nil;
        if (g_control != nil) {
            [g_control orderOut:nil];
            [g_control close];
            g_control = nil;
        }
    }
    return 0;
}

int m06_overlay_sync(OverlayOut *out) {
    if (out == NULL) {
        return 1;
    }
    memset(out, 0, sizeof *out);
    @autoreleasepool {
        if ((int)g_overlay.count != (int)[NSScreen screens].count) {
            rebuild_covers();
        }
        m06_overlay_wait_visible(2000);
        fill_overlay_out(out);
    }
    return 0;
}

int m06_overlay_escape_seen(void) {
    return atomic_load(&g_escape);
}

void m06_overlay_pump(uint32_t millis) {
    @autoreleasepool {
        pump(millis / 1000.0);
    }
}

int m06_control_open(void) {
    @autoreleasepool {
        if (g_control != nil) {
            return 0;
        }
        NSScreen *screen = [NSScreen mainScreen];
        NSRect frame_rect = screen != nil ? screen.frame : NSMakeRect(0, 0, 1024, 768);
        NSRect frame = NSMakeRect(NSMidX(frame_rect) + 8, NSMidY(frame_rect) - 120, 320, 240);
        NSWindow *window = [[NSWindow alloc] initWithContentRect:frame
                                                      styleMask:NSWindowStyleMaskBorderless
                                                        backing:NSBackingStoreBuffered
                                                          defer:NO];
        window.releasedWhenClosed = NO;
        window.opaque = YES;
        window.hasShadow = NO;
        window.level = NSScreenSaverWindowLevel;
        window.ignoresMouseEvents = YES;
        window.collectionBehavior = NSWindowCollectionBehaviorCanJoinAllSpaces |
                                    NSWindowCollectionBehaviorFullScreenAuxiliary;
        window.backgroundColor = [NSColor colorWithSRGBRed:0.05 green:0.82 blue:0.08 alpha:1];
        g_control = window;
        [window orderFrontRegardless];
        wait_until(2, ^{
          return (window.isVisible && (window.occlusionState & NSWindowOcclusionStateVisible) != 0)
                     ? 1
                     : 0;
        });
    }
    return 0;
}

int m06_target_open(ClickOut *out) {
    if (out == NULL) {
        return 1;
    }
    memset(out, 0, sizeof *out);
    atomic_store(&g_clicks, 0);
    atomic_store(&g_keys, 0);
    @autoreleasepool {
        ensure_app();
        NSScreen *screen = [NSScreen mainScreen];
        NSRect visible = screen != nil ? screen.visibleFrame : NSMakeRect(40, 40, 800, 600);
        NSRect frame = NSMakeRect(NSMinX(visible) + 48, NSMinY(visible) + 48, 280, 160);
        TargetWindow *window = [[TargetWindow alloc] initWithContentRect:frame
                                                               styleMask:NSWindowStyleMaskTitled
                                                                 backing:NSBackingStoreBuffered
                                                                   defer:NO];
        window.releasedWhenClosed = NO;
        window.title = @"m06 target";
        window.level = NSNormalWindowLevel;
        TargetView *view = [[TargetView alloc] initWithFrame:NSMakeRect(0, 0, 280, 160)];
        window.contentView = view;
        activate_app();
        [window makeKeyAndOrderFront:nil];
        [window makeFirstResponder:view];
        g_target = window;
        wait_until(2, ^{
          [window makeKeyAndOrderFront:nil];
          return window.isKeyWindow ? 1 : 0;
        });
        out->target_is_key = window.isKeyWindow ? 1 : 0;
        out->accessibility = AXIsProcessTrusted() ? 1 : 0;
        out->listen = CGPreflightListenEventAccess() ? 1 : 0;
    }
    return 0;
}

int m06_target_close(void) {
    @autoreleasepool {
        if (g_target != nil) {
            [g_target orderOut:nil];
            [g_target close];
            g_target = nil;
        }
        g_target_button = nil;
    }
    return 0;
}

int m06_posted_click_focus(ClickOut *out) {
    if (out == NULL) {
        return 1;
    }
    memset(out, 0, sizeof *out);
    @autoreleasepool {
        out->accessibility = AXIsProcessTrusted() ? 1 : 0;
        out->listen = CGPreflightListenEventAccess() ? 1 : 0;
        BOOL overlay_key = NO;
        for (NSWindow *window in g_overlay) {
            if (window.isKeyWindow) {
                overlay_key = YES;
            }
        }
        out->overlay_is_key = overlay_key ? 1 : 0;
        out->target_is_key = (g_target != nil && g_target.isKeyWindow) ? 1 : 0;
        if (g_target == nil) {
            snprintf(out->error, sizeof out->error, "target window missing");
            return 0;
        }
        if (out->accessibility == 0) {
            snprintf(out->error, sizeof out->error, "Accessibility not granted");
            out->click_count = atomic_load(&g_clicks);
            out->key_count = atomic_load(&g_keys);
            return 0;
        }
        activate_app();
        [g_target makeKeyAndOrderFront:nil];
        [g_target makeFirstResponder:g_target.contentView];
        BOOL became_key = wait_until(2, ^{
          [g_target makeKeyAndOrderFront:nil];
          return g_target.isKeyWindow ? 1 : 0;
        });
        if (!became_key) {
            snprintf(out->error, sizeof out->error, "target never became key");
            out->overlay_is_key = overlay_key ? 1 : 0;
            out->target_is_key = g_target.isKeyWindow ? 1 : 0;
            out->click_count = atomic_load(&g_clicks);
            out->key_count = atomic_load(&g_keys);
            return 0;
        }
        int baseline_clicks = atomic_load(&g_clicks);
        int baseline_keys = atomic_load(&g_keys);
        CGPoint point = quartz_point_in_view(g_target.contentView);
        CGEventRef down = CGEventCreateMouseEvent(NULL, kCGEventLeftMouseDown, point, kCGMouseButtonLeft);
        CGEventRef up = CGEventCreateMouseEvent(NULL, kCGEventLeftMouseUp, point, kCGMouseButtonLeft);
        if (down != NULL && up != NULL) {
            CGEventSetIntegerValueField(down, kCGMouseEventClickState, 1);
            CGEventSetIntegerValueField(up, kCGMouseEventClickState, 1);
            CGEventPost(kCGSessionEventTap, down);
            CGEventPost(kCGSessionEventTap, up);
        }
        if (down != NULL) {
            CFRelease(down);
        }
        if (up != NULL) {
            CFRelease(up);
        }
        CGEventSourceRef source = CGEventSourceCreate(kCGEventSourceStateCombinedSessionState);
        if (source != NULL) {
            CGEventRef key = CGEventCreateKeyboardEvent(source, (CGKeyCode)0, true);
            if (key != NULL) {
                CGEventPost(kCGSessionEventTap, key);
                CFRelease(key);
            }
            CFRelease(source);
        }
        wait_until(2, ^{
          return (atomic_load(&g_clicks) > baseline_clicks || atomic_load(&g_keys) > baseline_keys)
                     ? 1
                     : 0;
        });
        if (atomic_load(&g_clicks) <= baseline_clicks) {
            pid_t pid = getpid();
            CGEventRef hid_down =
                CGEventCreateMouseEvent(NULL, kCGEventLeftMouseDown, point, kCGMouseButtonLeft);
            CGEventRef hid_up =
                CGEventCreateMouseEvent(NULL, kCGEventLeftMouseUp, point, kCGMouseButtonLeft);
            if (hid_down != NULL && hid_up != NULL) {
                CGEventPostToPid(pid, hid_down);
                CGEventPostToPid(pid, hid_up);
            }
            if (hid_down != NULL) {
                CFRelease(hid_down);
            }
            if (hid_up != NULL) {
                CFRelease(hid_up);
            }
            wait_until(2, ^{
              return (atomic_load(&g_clicks) > baseline_clicks ||
                      atomic_load(&g_keys) > baseline_keys)
                         ? 1
                         : 0;
            });
        }
        overlay_key = NO;
        for (NSWindow *window in g_overlay) {
            if (window.isKeyWindow) {
                overlay_key = YES;
            }
        }
        out->overlay_is_key = overlay_key ? 1 : 0;
        out->target_is_key = g_target.isKeyWindow ? 1 : 0;
        out->click_count = atomic_load(&g_clicks);
        out->key_count = atomic_load(&g_keys);
    }
    return 0;
}

int m06_presentation_apply(PresentOut *out) {
    if (out == NULL) {
        return 1;
    }
    memset(out, 0, sizeof *out);
    @autoreleasepool {
        ensure_app();
        NSApplication *app = [NSApplication sharedApplication];
        out->app_active = app.isActive ? 1 : 0;
        NSApplicationPresentationOptions options = NSApplicationPresentationDisableProcessSwitching |
                                                   NSApplicationPresentationDisableForceQuit |
                                                   NSApplicationPresentationDisableSessionTermination;
        @try {
            app.presentationOptions = options;
        } @catch (NSException *exception) {
            snprintf(out->error, sizeof out->error, "presentationOptions exception");
        }
        pump(0.1);
        NSApplicationPresentationOptions current = app.currentSystemPresentationOptions;
        out->process_switching =
            (current & NSApplicationPresentationDisableProcessSwitching) != 0 ? 1 : 0;
        out->force_quit = (current & NSApplicationPresentationDisableForceQuit) != 0 ? 1 : 0;
        out->session_termination =
            (current & NSApplicationPresentationDisableSessionTermination) != 0 ? 1 : 0;
        out->app_active = app.isActive ? 1 : 0;
    }
    return 0;
}

int m06_presentation_clear(void) {
    @autoreleasepool {
        NSApplication *app = [NSApplication sharedApplication];
        app.presentationOptions = NSApplicationPresentationDefault;
    }
    return 0;
}

int m06_display_count(void) {
    @autoreleasepool {
        ensure_app();
        return (int)[NSScreen screens].count;
    }
}

int m06_hw_model(char *dest, size_t cap) {
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
        if (red >= 160 && green <= 80 && blue >= 90 && blue <= 230 && red > green + 40 && blue > green + 40) {
            pink++;
        }
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
        out->marker = -1;
        out->control = -1;
        return;
    }
    out->opened = 1;
    out->width = (int)CGImageGetWidth(image);
    out->height = (int)CGImageGetHeight(image);
    int nonblack = 0;
    count_pixels(image, &nonblack, &out->marker);
    out->control = g_pixel_control;
    CGImageRelease(image);
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

@interface StreamSink : NSObject <SCStreamOutput>
@end
@implementation StreamSink
- (void)stream:(SCStream *)stream
    didOutputSampleBuffer:(CMSampleBufferRef)sampleBuffer
                   ofType:(SCStreamOutputType)type {
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
    if (base == NULL || format != kCVPixelFormatType_32BGRA || width == 0 || height == 0 || width > 8192 ||
        height > 8192) {
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

static void capture_sckit(CaptureOut *out) {
    if (@available(macOS 12.3, *)) {
        memset(&g_sc, 0, sizeof g_sc);
        atomic_store(&g_sc_done, 0);
        int generation = atomic_fetch_add(&g_sc_gen, 1) + 1;
        [SCShareableContent getShareableContentWithCompletionHandler:^(SCShareableContent *content, NSError *error) {
          if (atomic_load(&g_sc_gen) != generation) {
              return;
          }
          if (error != nil || content.displays.count == 0) {
              if (error != nil) {
                  NSString *domain = error.domain != nil ? error.domain : @"unknown";
                  snprintf(g_sc.domain, sizeof g_sc.domain, "%s",
                           domain.UTF8String != NULL ? domain.UTF8String : "");
                  g_sc.error_code = (int)error.code;
              } else {
                  snprintf(g_sc.detail, sizeof g_sc.detail, "no displays");
              }
              atomic_store(&g_sc_done, 1);
              return;
          }
          SCDisplay *display = content.displays.firstObject;
          if (@available(macOS 14.0, *)) {
              SCContentFilter *filter = [[SCContentFilter alloc] initWithDisplay:display excludingWindows:@[]];
              SCStreamConfiguration *config = [[SCStreamConfiguration alloc] init];
              config.width = (size_t)display.frame.size.width;
              config.height = (size_t)display.frame.size.height;
              if (@available(macOS 13.0, *)) {
                  config.capturesAudio = NO;
              }
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
                                              snprintf(g_sc.detail, sizeof g_sc.detail, "SCScreenshotManager");
                                          } else {
                                              snprintf(g_sc.detail, sizeof g_sc.detail,
                                                       "SCScreenshotManager returned NULL");
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
        if (g_sc.opened) {
            out->opened = 1;
            out->width = g_sc.width;
            out->height = g_sc.height;
            out->marker = g_sc.marker;
            out->control = g_sc.control;
            return;
        }
        snprintf(out->detail, sizeof out->detail, "%s", g_sc.detail[0] ? g_sc.detail : "timed out");
        return;
    }
    snprintf(out->detail, sizeof out->detail, "unavailable");
}

static void capture_scstream(CaptureOut *out) {
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
              snprintf(g_sc.detail, sizeof g_sc.detail, error != nil ? "NSError" : "no displays");
              atomic_store(&g_sc_done, 1);
              return;
          }
          SCDisplay *display = content.displays.firstObject;
          SCContentFilter *filter = [[SCContentFilter alloc] initWithDisplay:display excludingWindows:@[]];
          SCStreamConfiguration *config = [[SCStreamConfiguration alloc] init];
          config.width = (size_t)display.frame.size.width;
          config.height = (size_t)display.frame.size.height;
          config.pixelFormat = kCVPixelFormatType_32BGRA;
          if (@available(macOS 13.0, *)) {
              config.capturesAudio = NO;
          }
          config.minimumFrameInterval = CMTimeMake(1, 2);
          config.queueDepth = 1;
          g_stream_sink = [[StreamSink alloc] init];
          g_live_stream = [[SCStream alloc] initWithFilter:filter configuration:config delegate:nil];
          NSError *add_error = nil;
          BOOL added = [g_live_stream addStreamOutput:g_stream_sink
                                                 type:SCStreamOutputTypeScreen
                                  sampleHandlerQueue:dispatch_get_main_queue()
                                               error:&add_error];
          if (!added || add_error != nil) {
              snprintf(g_sc.detail, sizeof g_sc.detail, "addStreamOutput");
              atomic_store(&g_sc_done, 1);
              return;
          }
          [g_live_stream startCaptureWithCompletionHandler:^(NSError *start_error) {
            if (start_error != nil && atomic_load(&g_sc_done) == 0) {
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
        if (g_sc.opened) {
            out->opened = 1;
            out->width = g_sc.width;
            out->height = g_sc.height;
            out->marker = g_sc.marker;
            out->control = g_sc.control;
            return;
        }
        snprintf(out->detail, sizeof out->detail, "%s", g_sc.detail[0] ? g_sc.detail : "SCStream timed out");
        return;
    }
    snprintf(out->detail, sizeof out->detail, "unavailable");
}

static void capture_screencapture(CaptureOut *out) {
    char path[] = "/tmp/m06-overlay-capture.png";
    unlink(path);
    NSTask *task = [[NSTask alloc] init];
    task.executableURL = [NSURL fileURLWithPath:@"/usr/sbin/screencapture"];
    task.arguments = @[ @"-x", @"/tmp/m06-overlay-capture.png" ];
    NSError *launch_error = nil;
    if (![task launchAndReturnError:&launch_error]) {
        snprintf(out->detail, sizeof out->detail, "screencapture launch failed");
        return;
    }
    NSDate *until = [NSDate dateWithTimeIntervalSinceNow:8];
    while (task.running && [until timeIntervalSinceNow] > 0) {
        pump(0.05);
    }
    if (task.running) {
        [task terminate];
        snprintf(out->detail, sizeof out->detail, "screencapture timed out");
        return;
    }
    int rc = task.terminationStatus;
    if (rc != 0) {
        snprintf(out->detail, sizeof out->detail, "screencapture exit %d", rc);
        return;
    }
    NSImage *image = [[NSImage alloc] initWithContentsOfFile:[NSString stringWithUTF8String:path]];
    if (image == nil) {
        snprintf(out->detail, sizeof out->detail, "screencapture png missing");
        return;
    }
    NSRect rect = NSMakeRect(0, 0, image.size.width, image.size.height);
    CGImageRef cg = [image CGImageForProposedRect:&rect context:nil hints:nil];
    if (cg == NULL) {
        snprintf(out->detail, sizeof out->detail, "CGImageForProposedRect returned NULL");
        return;
    }
    CGImageRetain(cg);
    capture_from_image(cg, out, "screencapture image NULL");
    unlink(path);
}

int m06_capture(const char *api, CaptureOut *out) {
    if (out == NULL || api == NULL) {
        return 1;
    }
    memset(out, 0, sizeof *out);
    out->marker = -1;
    out->control = -1;
    @autoreleasepool {
        if (strcmp(api, "CGWindowListCreateImage") == 0) {
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
        if (strcmp(api, "CGDisplayCreateImage") == 0) {
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
        if (strcmp(api, "screencapture") == 0) {
            capture_screencapture(out);
            return 0;
        }
        if (strcmp(api, "ScreenCaptureKit") == 0) {
            capture_sckit(out);
            return 0;
        }
        if (strcmp(api, "SCStream") == 0) {
            capture_scstream(out);
            return 0;
        }
        snprintf(out->detail, sizeof out->detail, "unknown capture api");
    }
    return 0;
}
