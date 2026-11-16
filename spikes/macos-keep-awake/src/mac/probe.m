// Throwaway m07 keep-awake kit. Does not lock the session.
// Does not read hardware identifiers. Overlay ignores the mouse and is not key.

#import <AppKit/AppKit.h>
#import <Foundation/Foundation.h>
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
    char error[256];
} WindowOut;

typedef struct {
    uint64_t ticks;
    char error[256];
} TimerOut;

static NSMutableArray<NSWindow *> *g_overlay = nil;
static NSWindow *g_timer = nil;
static NSTextField *g_timer_label = nil;
static uint64_t g_ticks = 0;
static atomic_int g_escape = 0;
static id g_escape_monitor = nil;

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

int m07_overlay_open(WindowOut *out) {
    if (out == NULL) {
        return 1;
    }
    memset(out, 0, sizeof *out);
    @autoreleasepool {
        ensure_app();
        NSArray<NSScreen *> *screens = [NSScreen screens];
        out->display_count = (int)screens.count;
        g_overlay = [NSMutableArray array];
        for (NSScreen *screen in screens) {
            CoverWindow *window = [[CoverWindow alloc] initWithContentRect:screen.frame
                                                                styleMask:NSWindowStyleMaskBorderless
                                                                  backing:NSBackingStoreBuffered
                                                                    defer:NO];
            window.releasedWhenClosed = NO;
            window.opaque = NO;
            window.hasShadow = NO;
            window.ignoresMouseEvents = YES;
            window.level = NSStatusWindowLevel;
            window.backgroundColor = [[NSColor blackColor] colorWithAlphaComponent:0.35];
            window.collectionBehavior = NSWindowCollectionBehaviorCanJoinAllSpaces |
                                        NSWindowCollectionBehaviorFullScreenAuxiliary;
            [window orderFrontRegardless];
            [g_overlay addObject:window];
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
    }
    return 0;
}
