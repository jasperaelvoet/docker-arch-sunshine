#define _GNU_SOURCE

#include <dirent.h>
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <gio/gio.h>
#include <gio/gunixfdlist.h>
#include <libei.h>
#include <libevdev/libevdev.h>
#include <libevdev/libevdev-uinput.h>
#include <linux/input.h>
#include <limits.h>
#include <pthread.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <sys/time.h>
#include <time.h>
#include <unistd.h>

#define MAX_EI_DEVICES 64
#define KEY_STATE_BYTES ((KEY_MAX + 8) / 8)
#define MOMENTARY_ABS_AXES 4
#define INPUT_FPS_STATE_FILE "arch-sunshine-input-fps"
#define DEFAULT_INPUT_FPS 60.0
#define MIN_DYNAMIC_INPUT_HOLD_MS 4
#define MAX_DYNAMIC_INPUT_HOLD_MS 50
#define MAX_INPUT_HOLD_OVERRIDE_MS 250
#define INPUT_HOLD_REFRESH_SECONDS 0.25
#define PORTAL_DEVICE_KEYBOARD (1 << 0)
#define PORTAL_DEVICE_POINTER (1 << 1)
#define PORTAL_DEVICE_TOUCHSCREEN (1 << 2)
#define PORTAL_DEVICE_ALL (PORTAL_DEVICE_KEYBOARD | PORTAL_DEVICE_POINTER | PORTAL_DEVICE_TOUCHSCREEN)

struct input_device {
    const struct libevdev_uinput *uinput;
    bool pointer;
    bool keyboard;
    bool absolute_pointer;
    bool swap_primary_buttons;
    int32_t rel_x;
    int32_t rel_y;
    int32_t scroll_x;
    int32_t scroll_y;
    bool have_abs_x;
    bool have_abs_y;
    bool abs_dirty;
    int32_t abs_x;
    int32_t abs_y;
    int32_t abs_min_x;
    int32_t abs_max_x;
    int32_t abs_min_y;
    int32_t abs_max_y;
    int32_t abs_min_z;
    int32_t abs_max_z;
    int32_t abs_min_rz;
    int32_t abs_max_rz;
    int32_t momentary_abs_state[MOMENTARY_ABS_AXES];
    double momentary_abs_pressed_at[MOMENTARY_ABS_AXES];
    uint8_t key_state[KEY_STATE_BYTES];
    double key_pressed_at[KEY_MAX + 1];
    struct input_device *next;
};

struct eis_sender {
    GDBusConnection *bus;
    GVariant *cookie;
    struct ei *ei;
    int fd;
    struct ei_device *devices[MAX_EI_DEVICES];
    size_t device_count;
    uint32_t sequence;
    struct ei_device *pointer;
    struct ei_device *pointer_abs;
    struct ei_device *button;
    struct ei_device *scroll;
    struct ei_device *keyboard;
    uint32_t region_x;
    uint32_t region_y;
    uint32_t region_width;
    uint32_t region_height;
};

static pthread_mutex_t state_lock = PTHREAD_MUTEX_INITIALIZER;
static struct input_device *devices;
static struct eis_sender sender;
static bool sender_initialized;
static bool sender_connected;
static double last_connect_attempt;

static int (*real_libevdev_uinput_create_from_device)(const struct libevdev *, int, struct libevdev_uinput **);
static int (*real_libevdev_uinput_write_event)(const struct libevdev_uinput *, unsigned int, unsigned int, int);
static void (*real_libevdev_uinput_destroy)(struct libevdev_uinput *);

static double monotonic_seconds(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (double)ts.tv_sec + (double)ts.tv_nsec / 1000000000.0;
}

static bool starts_with(const char *text, const char *prefix) {
    return text && strncmp(text, prefix, strlen(prefix)) == 0;
}

static bool env_truthy(const char *name, const char *fallback) {
    const char *value = getenv(name);
    if (!value) {
        value = fallback;
    }
    while (*value == ' ' || *value == '\t') {
        value++;
    }
    return !(*value == '\0' ||
             strcmp(value, "0") == 0 ||
             strcasecmp(value, "false") == 0 ||
             strcasecmp(value, "no") == 0 ||
             strcasecmp(value, "off") == 0);
}

static bool proxy_enabled(void) {
    return env_truthy("ARCH_SUNSHINE_LIBEI_INPUT", "1");
}

static bool parse_int_clamped(const char *value, int minimum, int maximum, int *out) {
    errno = 0;
    char *end = NULL;
    long parsed = strtol(value, &end, 10);
    while (end && (*end == ' ' || *end == '\t')) {
        end++;
    }
    if (errno != 0 || end == value || (end && *end != '\0')) {
        return false;
    }
    if (parsed < minimum) {
        parsed = minimum;
    }
    if (parsed > maximum) {
        parsed = maximum;
    }
    *out = (int)parsed;
    return true;
}

static bool env_int_clamped(const char *name, int minimum, int maximum, int *out) {
    const char *value = getenv(name);
    if (!value || *value == '\0') {
        return false;
    }
    return parse_int_clamped(value, minimum, maximum, out);
}

static bool parse_fps(const char *value, double *out) {
    if (!value || *value == '\0') {
        return false;
    }
    errno = 0;
    char *end = NULL;
    double parsed = strtod(value, &end);
    while (end && (*end == ' ' || *end == '\t' || *end == '\n' || *end == '\r')) {
        end++;
    }
    if (errno != 0 || end == value || (end && *end != '\0') || parsed <= 0.0 || parsed > 240.0) {
        return false;
    }
    *out = parsed;
    return true;
}

static bool input_fps_state_path(char *path, size_t size) {
    const char *override = getenv("ARCH_SUNSHINE_INPUT_FPS_FILE");
    if (override && *override) {
        int written = snprintf(path, size, "%s", override);
        return written > 0 && (size_t)written < size;
    }

    const char *runtime_dir = getenv("XDG_RUNTIME_DIR");
    if (!runtime_dir || !*runtime_dir) {
        runtime_dir = "/run/user/1000";
    }
    int written = snprintf(path, size, "%s/%s", runtime_dir, INPUT_FPS_STATE_FILE);
    return written > 0 && (size_t)written < size;
}

static double input_fps(void) {
    double fps = 0.0;
    if (parse_fps(getenv("ARCH_SUNSHINE_INPUT_FPS"), &fps)) {
        return fps;
    }

    char path[PATH_MAX];
    if (input_fps_state_path(path, sizeof(path))) {
        FILE *file = fopen(path, "r");
        if (file) {
            char text[64];
            if (fgets(text, sizeof(text), file) && parse_fps(text, &fps)) {
                fclose(file);
                return fps;
            }
            fclose(file);
        }
    }

    if (parse_fps(getenv("SUNSHINE_CLIENT_FPS"), &fps) ||
        parse_fps(getenv("SUNSHINE_FPS"), &fps)) {
        return fps;
    }

    return DEFAULT_INPUT_FPS;
}

static int dynamic_input_hold_ms(void) {
    double frame_ms = 1000.0 / input_fps();
    int hold_ms = (int)frame_ms;
    if ((double)hold_ms < frame_ms) {
        hold_ms++;
    }
    hold_ms++;
    if (hold_ms < MIN_DYNAMIC_INPUT_HOLD_MS) {
        hold_ms = MIN_DYNAMIC_INPUT_HOLD_MS;
    }
    if (hold_ms > MAX_DYNAMIC_INPUT_HOLD_MS) {
        hold_ms = MAX_DYNAMIC_INPUT_HOLD_MS;
    }
    return hold_ms;
}

static int input_min_hold_ms(void) {
    static int cached = -1;
    static double last_refresh = 0.0;

    int override = 0;
    if (env_int_clamped("ARCH_SUNSHINE_INPUT_MIN_HOLD_MS", 0, MAX_INPUT_HOLD_OVERRIDE_MS, &override)) {
        return override;
    }

    double now = monotonic_seconds();
    if (cached < 0 || now - last_refresh >= INPUT_HOLD_REFRESH_SECONDS) {
        cached = dynamic_input_hold_ms();
        last_refresh = now;
    }
    return cached;
}

static void sleep_seconds(double seconds) {
    if (seconds <= 0.0) {
        return;
    }

    struct timespec remaining = {
        .tv_sec = (time_t)seconds,
        .tv_nsec = (long)((seconds - (time_t)seconds) * 1000000000.0),
    };
    while (nanosleep(&remaining, &remaining) != 0 && errno == EINTR) {
    }
}

static void apply_min_hold(double pressed_at) {
    int min_hold_ms = input_min_hold_ms();
    if (min_hold_ms <= 0 || pressed_at <= 0.0) {
        return;
    }

    double elapsed = monotonic_seconds() - pressed_at;
    double minimum = (double)min_hold_ms / 1000.0;
    if (elapsed < minimum) {
        sleep_seconds(minimum - elapsed);
    }
}

static void log_message(const char *message) {
    fprintf(stderr, "arch-sunshine-libei-input: %s\n", message);
    fflush(stderr);
}

static void log_error(const char *prefix, const char *message) {
    fprintf(stderr, "arch-sunshine-libei-input: %s: %s\n", prefix, message);
    fflush(stderr);
}

static bool resolve_real_symbols(void) {
    if (!real_libevdev_uinput_create_from_device) {
        real_libevdev_uinput_create_from_device = dlsym(RTLD_NEXT, "libevdev_uinput_create_from_device");
    }
    if (!real_libevdev_uinput_write_event) {
        real_libevdev_uinput_write_event = dlsym(RTLD_NEXT, "libevdev_uinput_write_event");
    }
    if (!real_libevdev_uinput_destroy) {
        real_libevdev_uinput_destroy = dlsym(RTLD_NEXT, "libevdev_uinput_destroy");
    }
    return real_libevdev_uinput_create_from_device &&
           real_libevdev_uinput_write_event &&
           real_libevdev_uinput_destroy;
}

static bool key_state_changed(struct input_device *device, uint32_t code, bool pressed) {
    if (code > KEY_MAX) {
        return true;
    }

    uint8_t mask = (uint8_t)(1u << (code % 8));
    uint8_t *state = &device->key_state[code / 8];
    bool was_pressed = (*state & mask) != 0;

    if (was_pressed == pressed) {
        return false;
    }
    if (pressed) {
        *state |= mask;
        device->key_pressed_at[code] = monotonic_seconds();
    } else {
        apply_min_hold(device->key_pressed_at[code]);
        *state &= (uint8_t)~mask;
        device->key_pressed_at[code] = 0.0;
    }
    return true;
}

static bool momentary_abs_axis(const struct input_device *device, uint32_t code, int *index, int32_t *neutral, bool *digital) {
    *neutral = 0;
    *digital = false;
    if (code == ABS_HAT0X) {
        *index = 0;
        *digital = true;
        return true;
    }
    if (code == ABS_HAT0Y) {
        *index = 1;
        *digital = true;
        return true;
    }
    if (code == ABS_Z) {
        *index = 2;
        *neutral = device->abs_min_z;
        return true;
    }
    if (code == ABS_RZ) {
        *index = 3;
        *neutral = device->abs_min_rz;
        return true;
    }
    return false;
}

static void hold_momentary_abs_release(struct input_device *device, uint32_t code, int32_t value) {
    int index = -1;
    int32_t neutral = 0;
    bool digital = false;
    if (!momentary_abs_axis(device, code, &index, &neutral, &digital) ||
        device->momentary_abs_state[index] == value) {
        return;
    }

    bool was_pressed = device->momentary_abs_state[index] != neutral;
    bool pressed = value != neutral;
    if (!was_pressed && pressed) {
        device->momentary_abs_pressed_at[index] = monotonic_seconds();
    } else if (was_pressed && pressed) {
        if (digital) {
            apply_min_hold(device->momentary_abs_pressed_at[index]);
        }
        device->momentary_abs_pressed_at[index] = monotonic_seconds();
    } else if (was_pressed && !pressed) {
        apply_min_hold(device->momentary_abs_pressed_at[index]);
        device->momentary_abs_pressed_at[index] = 0.0;
    }
    device->momentary_abs_state[index] = value;
}

static double clamp_double(double value, double minimum, double maximum) {
    if (value < minimum) {
        return minimum;
    }
    if (value > maximum) {
        return maximum;
    }
    return value;
}

static void eis_sender_init(struct eis_sender *target) {
    memset(target, 0, sizeof(*target));
    target->fd = -1;
    target->sequence = 1;
    target->region_width = 1920;
    target->region_height = 1080;
}

static void eis_sender_classify_device(struct eis_sender *target, struct ei_device *device) {
    if (ei_device_has_capability(device, EI_DEVICE_CAP_POINTER)) {
        target->pointer = device;
    }
    if (ei_device_has_capability(device, EI_DEVICE_CAP_POINTER_ABSOLUTE)) {
        target->pointer_abs = device;
        struct ei_region *region = ei_device_get_region(device, 0);
        if (region) {
            target->region_x = ei_region_get_x(region);
            target->region_y = ei_region_get_y(region);
            target->region_width = ei_region_get_width(region);
            target->region_height = ei_region_get_height(region);
        }
    }
    if (ei_device_has_capability(device, EI_DEVICE_CAP_BUTTON)) {
        target->button = device;
    }
    if (ei_device_has_capability(device, EI_DEVICE_CAP_SCROLL)) {
        target->scroll = device;
    }
    if (ei_device_has_capability(device, EI_DEVICE_CAP_KEYBOARD)) {
        target->keyboard = device;
    }
}

static void eis_sender_classify_all(struct eis_sender *target) {
    target->pointer = NULL;
    target->pointer_abs = NULL;
    target->button = NULL;
    target->scroll = NULL;
    target->keyboard = NULL;
    for (size_t i = 0; i < target->device_count; i++) {
        eis_sender_classify_device(target, target->devices[i]);
    }
}

static void eis_sender_remove_device(struct eis_sender *target, struct ei_device *device) {
    for (size_t i = 0; i < target->device_count; i++) {
        if (target->devices[i] != device) {
            continue;
        }
        ei_device_unref(target->devices[i]);
        if (i + 1 < target->device_count) {
            memmove(&target->devices[i], &target->devices[i + 1], (target->device_count - i - 1) * sizeof(target->devices[0]));
        }
        target->device_count--;
        break;
    }
    eis_sender_classify_all(target);
}

static bool eis_sender_drain_events(struct eis_sender *target) {
    bool disconnected = false;
    struct ei_event *event;
    while ((event = ei_get_event(target->ei)) != NULL) {
        enum ei_event_type type = ei_event_get_type(event);
        switch (type) {
        case EI_EVENT_DISCONNECT:
            disconnected = true;
            break;
        case EI_EVENT_SEAT_ADDED: {
            struct ei_seat *seat = ei_event_get_seat(event);
            ei_seat_bind_capabilities(
                seat,
                EI_DEVICE_CAP_POINTER,
                EI_DEVICE_CAP_POINTER_ABSOLUTE,
                EI_DEVICE_CAP_KEYBOARD,
                EI_DEVICE_CAP_SCROLL,
                EI_DEVICE_CAP_BUTTON,
                NULL);
            break;
        }
        case EI_EVENT_DEVICE_ADDED: {
            struct ei_device *device = ei_event_get_device(event);
            if (device && target->device_count < MAX_EI_DEVICES) {
                target->devices[target->device_count++] = ei_device_ref(device);
                eis_sender_classify_device(target, device);
            }
            break;
        }
        case EI_EVENT_DEVICE_RESUMED: {
            struct ei_device *device = ei_event_get_device(event);
            if (device) {
                ei_device_start_emulating(device, target->sequence);
                target->sequence++;
                if (target->sequence == 0) {
                    target->sequence = 1;
                }
            }
            break;
        }
        case EI_EVENT_DEVICE_REMOVED:
            eis_sender_remove_device(target, ei_event_get_device(event));
            break;
        default:
            break;
        }
        ei_event_unref(event);
        if (disconnected) {
            break;
        }
    }
    return disconnected;
}

static bool eis_sender_pump(struct eis_sender *target, double timeout) {
    double deadline = monotonic_seconds() + timeout;
    bool disconnected = false;
    for (;;) {
        double wait = deadline - monotonic_seconds();
        if (wait < 0) {
            wait = 0;
        }
        if (wait > 0.05) {
            wait = 0.05;
        }

        if (target->fd >= 0) {
            fd_set read_fds;
            FD_ZERO(&read_fds);
            FD_SET(target->fd, &read_fds);
            struct timeval tv = {
                .tv_sec = (time_t)wait,
                .tv_usec = (suseconds_t)((wait - (time_t)wait) * 1000000.0),
            };
            int ready = select(target->fd + 1, &read_fds, NULL, NULL, &tv);
            if (ready > 0 && FD_ISSET(target->fd, &read_fds)) {
                ei_dispatch(target->ei);
            }
        }

        if (eis_sender_drain_events(target)) {
            disconnected = true;
            break;
        }
        if (monotonic_seconds() >= deadline) {
            break;
        }
    }
    return disconnected;
}

static int connect_to_eis_fd(struct eis_sender *target, GError **error) {
    target->bus = g_bus_get_sync(G_BUS_TYPE_SESSION, NULL, error);
    if (!target->bus) {
        return -1;
    }

    GUnixFDList *fd_list = NULL;
    GVariant *reply = g_dbus_connection_call_with_unix_fd_list_sync(
        target->bus,
        "org.kde.KWin",
        "/org/kde/KWin/EIS/RemoteDesktop",
        "org.kde.KWin.EIS.RemoteDesktop",
        "connectToEIS",
        g_variant_new("(i)", PORTAL_DEVICE_ALL),
        NULL,
        G_DBUS_CALL_FLAGS_NONE,
        -1,
        NULL,
        &fd_list,
        NULL,
        error);
    if (!reply) {
        return -1;
    }

    GVariant *handle_value = g_variant_get_child_value(reply, 0);
    GVariant *cookie_value = g_variant_get_child_value(reply, 1);
    int handle = g_variant_get_handle(handle_value);
    int fd = g_unix_fd_list_get(fd_list, handle, error);

    g_variant_unref(handle_value);
    if (fd >= 0) {
        target->cookie = g_variant_ref_sink(cookie_value);
    }
    g_variant_unref(cookie_value);
    g_variant_unref(reply);
    g_object_unref(fd_list);
    return fd;
}

static bool eis_sender_connect(struct eis_sender *target, GError **error) {
    int fd = connect_to_eis_fd(target, error);
    if (fd < 0) {
        return false;
    }

    target->ei = ei_new_sender(NULL);
    if (!target->ei) {
        close(fd);
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_FAILED, "failed to create libei sender");
        return false;
    }

    ei_configure_name(target->ei, "arch-sunshine-libei-input");
    int result = ei_setup_backend_fd(target->ei, fd);
    if (result != 0) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_FAILED, "ei_setup_backend_fd failed: %s", strerror(-result));
        return false;
    }
    target->fd = ei_get_fd(target->ei);
    if (eis_sender_pump(target, 3.0)) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_CLOSED, "KWin EIS disconnected during setup");
        return false;
    }
    if (!target->keyboard && !target->pointer && !target->pointer_abs && !target->button) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_FAILED, "KWin EIS did not expose any input devices");
        return false;
    }
    return true;
}

static void eis_sender_close(struct eis_sender *target) {
    if (target->bus && target->cookie) {
        GVariant *children[] = {target->cookie};
        GError *error = NULL;
        GVariant *reply = g_dbus_connection_call_sync(
            target->bus,
            "org.kde.KWin",
            "/org/kde/KWin/EIS/RemoteDesktop",
            "org.kde.KWin.EIS.RemoteDesktop",
            "disconnect",
            g_variant_new_tuple(children, 1),
            NULL,
            G_DBUS_CALL_FLAGS_NONE,
            500,
            NULL,
            &error);
        if (reply) {
            g_variant_unref(reply);
        }
        if (error) {
            g_error_free(error);
        }
    }
    for (size_t i = 0; i < target->device_count; i++) {
        ei_device_unref(target->devices[i]);
    }
    if (target->ei) {
        ei_unref(target->ei);
    }
    if (target->cookie) {
        g_variant_unref(target->cookie);
    }
    if (target->bus) {
        g_object_unref(target->bus);
    }
    eis_sender_init(target);
}

static bool ensure_eis_connected(void) {
    if (!sender_initialized) {
        eis_sender_init(&sender);
        sender_initialized = true;
    }
    if (sender_connected) {
        if (!eis_sender_pump(&sender, 0.0)) {
            return true;
        }
        log_message("KWin EIS disconnected; reconnecting");
        eis_sender_close(&sender);
        sender_connected = false;
        last_connect_attempt = 0.0;
    }

    double now = monotonic_seconds();
    if (now - last_connect_attempt < 1.0) {
        return false;
    }
    last_connect_attempt = now;

    GError *error = NULL;
    if (!eis_sender_connect(&sender, &error)) {
        if (error) {
            log_error("failed to connect to KWin EIS", error->message);
            g_error_free(error);
        } else {
            log_message("failed to connect to KWin EIS");
        }
        eis_sender_close(&sender);
        return false;
    }

    sender_connected = true;
    log_message("connected to KWin EIS");
    return true;
}

static void eis_frame(struct ei_device *device) {
    if (!device) {
        return;
    }
    ei_device_frame(device, ei_now(sender.ei));
    ei_dispatch(sender.ei);
    if (eis_sender_pump(&sender, 0.0)) {
        log_message("KWin EIS disconnected while sending input; reconnecting");
        eis_sender_close(&sender);
        sender_connected = false;
        last_connect_attempt = 0.0;
    }
}

static void eis_move_relative(int32_t dx, int32_t dy) {
    if (sender.pointer && (dx || dy)) {
        ei_device_pointer_motion(sender.pointer, (double)dx, (double)dy);
        eis_frame(sender.pointer);
    }
}

static void eis_move_absolute(struct input_device *device) {
    if (!sender.pointer_abs || !device->have_abs_x || !device->have_abs_y || !device->abs_dirty) {
        return;
    }
    if (device->abs_max_x <= device->abs_min_x ||
        device->abs_max_y <= device->abs_min_y ||
        sender.region_width == 0 ||
        sender.region_height == 0) {
        return;
    }

    double target_x = (double)sender.region_x +
                      ((double)(device->abs_x - device->abs_min_x) /
                       (double)(device->abs_max_x - device->abs_min_x)) *
                          (double)(sender.region_width - 1);
    double target_y = (double)sender.region_y +
                      ((double)(device->abs_y - device->abs_min_y) /
                       (double)(device->abs_max_y - device->abs_min_y)) *
                          (double)(sender.region_height - 1);
    target_x = clamp_double(target_x, sender.region_x, sender.region_x + sender.region_width - 1);
    target_y = clamp_double(target_y, sender.region_y, sender.region_y + sender.region_height - 1);
    ei_device_pointer_motion_absolute(sender.pointer_abs, target_x, target_y);
    eis_frame(sender.pointer_abs);
    device->abs_dirty = false;
}

static struct ei_device *eis_pointer_device_for_source(const struct input_device *device) {
    if (device->absolute_pointer && sender.pointer_abs) {
        return sender.pointer_abs;
    }
    if (!device->absolute_pointer && sender.pointer) {
        return sender.pointer;
    }
    return sender.button ? sender.button : sender.scroll;
}

static void eis_button(const struct input_device *device, uint32_t code, bool pressed) {
    struct ei_device *target = eis_pointer_device_for_source(device);
    if (target && ei_device_has_capability(target, EI_DEVICE_CAP_BUTTON)) {
        ei_device_button_button(target, code, pressed);
        eis_frame(target);
    }
}

static void eis_scroll(const struct input_device *device, int32_t dx, int32_t dy) {
    struct ei_device *target = eis_pointer_device_for_source(device);
    if (target && ei_device_has_capability(target, EI_DEVICE_CAP_SCROLL) && (dx || dy)) {
        ei_device_scroll_discrete(target, dx, dy);
        eis_frame(target);
    }
}

static void eis_key(uint32_t code, bool pressed) {
    if (sender.keyboard) {
        ei_device_keyboard_key(sender.keyboard, code, pressed);
        eis_frame(sender.keyboard);
    }
}

static void flush_device(struct input_device *device) {
    if (device->rel_x || device->rel_y) {
        eis_move_relative(device->rel_x, device->rel_y);
        device->rel_x = 0;
        device->rel_y = 0;
    }
    eis_move_absolute(device);
    if (device->scroll_x || device->scroll_y) {
        eis_scroll(device, device->scroll_x, device->scroll_y);
        device->scroll_x = 0;
        device->scroll_y = 0;
    }
}

static void discard_pointer_frame(struct input_device *device) {
    device->rel_x = 0;
    device->rel_y = 0;
    device->scroll_x = 0;
    device->scroll_y = 0;
    device->abs_dirty = false;
}

static void handle_event(struct input_device *device, unsigned int type, unsigned int code, int value) {
    if (type == EV_ABS) {
        hold_momentary_abs_release(device, code, value);
    }

    if (type == EV_REL && device->pointer) {
        if (code == REL_X) {
            device->rel_x += value;
        } else if (code == REL_Y) {
            device->rel_y += value;
        } else if (code == REL_HWHEEL_HI_RES) {
            device->scroll_x += value;
        } else if (code == REL_WHEEL_HI_RES) {
            device->scroll_y -= value;
        } else if (code == REL_HWHEEL) {
            device->scroll_x += value * 120;
        } else if (code == REL_WHEEL) {
            device->scroll_y -= value * 120;
        }
    } else if (type == EV_ABS && device->pointer) {
        if (code == ABS_X) {
            device->abs_x = value;
            device->have_abs_x = true;
            device->abs_dirty = true;
        } else if (code == ABS_Y) {
            device->abs_y = value;
            device->have_abs_y = true;
            device->abs_dirty = true;
        }
    } else if (type == EV_KEY) {
        if (value != 0 && value != 1) {
            return;
        }
        bool pressed = value == 1;
        if (!key_state_changed(device, code, pressed)) {
            return;
        }
        if (!device->pointer && !device->keyboard) {
            return;
        }
        if (!ensure_eis_connected()) {
            return;
        }
        if (code >= BTN_MOUSE && code < BTN_JOYSTICK) {
            if (!device->pointer) {
                return;
            }
            if (device->swap_primary_buttons) {
                if (code == BTN_LEFT) {
                    code = BTN_RIGHT;
                } else if (code == BTN_RIGHT) {
                    code = BTN_LEFT;
                }
            }
            eis_button(device, code, pressed);
        } else if (device->keyboard) {
            eis_key(code, pressed);
        }
    } else if (type == EV_SYN && code == SYN_REPORT) {
        if (!device->pointer) {
            return;
        }
        if (!ensure_eis_connected()) {
            discard_pointer_frame(device);
            return;
        }
        flush_device(device);
    }
}

static struct input_device *find_device(const struct libevdev_uinput *uinput) {
    for (struct input_device *device = devices; device; device = device->next) {
        if (device->uinput == uinput) {
            return device;
        }
    }
    return NULL;
}

static void read_abs_info(const struct libevdev *evdev, int code, int32_t *minimum, int32_t *maximum) {
    const struct input_absinfo *info = libevdev_get_abs_info(evdev, code);
    if (info) {
        *minimum = info->minimum;
        *maximum = info->maximum;
        return;
    }
    *minimum = 0;
    *maximum = 65535;
}

static void add_device(const struct libevdev *evdev, const struct libevdev_uinput *uinput) {
    const char *name = libevdev_get_name(evdev);
    bool pointer = starts_with(name, "Mouse passthrough") ||
                   starts_with(name, "Arch Sunshine Libei Mouse Test");
    bool keyboard = starts_with(name, "Keyboard passthrough") ||
                    starts_with(name, "Arch Sunshine Libei Keyboard Test");

    struct input_device *device = calloc(1, sizeof(*device));
    if (!device) {
        return;
    }
    device->uinput = uinput;
    device->pointer = pointer;
    device->keyboard = keyboard;
    device->absolute_pointer = pointer &&
                               libevdev_has_event_code(evdev, EV_ABS, ABS_X) &&
                               libevdev_has_event_code(evdev, EV_ABS, ABS_Y);
    device->swap_primary_buttons = pointer && env_truthy("SUNSHINE_SWAP_PRIMARY_MOUSE_BUTTONS", "1");
    read_abs_info(evdev, ABS_X, &device->abs_min_x, &device->abs_max_x);
    read_abs_info(evdev, ABS_Y, &device->abs_min_y, &device->abs_max_y);
    read_abs_info(evdev, ABS_Z, &device->abs_min_z, &device->abs_max_z);
    read_abs_info(evdev, ABS_RZ, &device->abs_min_rz, &device->abs_max_rz);
    device->momentary_abs_state[2] = device->abs_min_z;
    device->momentary_abs_state[3] = device->abs_min_rz;
    device->next = devices;
    devices = device;

    if (!pointer && !keyboard) {
        return;
    }

    if (pointer && keyboard) {
        log_error("proxying virtual pointer and keyboard", name);
    } else if (pointer) {
        log_error("proxying virtual pointer", name);
    } else {
        log_error("proxying virtual keyboard", name);
    }
    ensure_eis_connected();
}

static void remove_device(const struct libevdev_uinput *uinput) {
    struct input_device **link = &devices;
    while (*link) {
        struct input_device *device = *link;
        if (device->uinput != uinput) {
            link = &device->next;
            continue;
        }
        *link = device->next;
        free(device);
        return;
    }
}

int libevdev_uinput_create_from_device(const struct libevdev *evdev, int uinput_fd, struct libevdev_uinput **uinput_dev) {
    if (!resolve_real_symbols()) {
        errno = ENOSYS;
        return -ENOSYS;
    }

    int result = real_libevdev_uinput_create_from_device(evdev, uinput_fd, uinput_dev);
    if (result != 0 || !proxy_enabled() || !uinput_dev || !*uinput_dev) {
        return result;
    }

    pthread_mutex_lock(&state_lock);
    add_device(evdev, *uinput_dev);
    pthread_mutex_unlock(&state_lock);
    return result;
}

int libevdev_uinput_write_event(const struct libevdev_uinput *uinput_dev, unsigned int type, unsigned int code, int value) {
    if (!resolve_real_symbols()) {
        errno = ENOSYS;
        return -ENOSYS;
    }

    if (proxy_enabled()) {
        pthread_mutex_lock(&state_lock);
        struct input_device *device = find_device(uinput_dev);
        if (device) {
            handle_event(device, type, code, value);
        }
        int result = real_libevdev_uinput_write_event(uinput_dev, type, code, value);
        pthread_mutex_unlock(&state_lock);
        return result;
    }

    return real_libevdev_uinput_write_event(uinput_dev, type, code, value);
}

void libevdev_uinput_destroy(struct libevdev_uinput *uinput_dev) {
    if (!resolve_real_symbols()) {
        return;
    }

    if (proxy_enabled()) {
        pthread_mutex_lock(&state_lock);
        remove_device(uinput_dev);
        pthread_mutex_unlock(&state_lock);
    }
    real_libevdev_uinput_destroy(uinput_dev);
}

__attribute__((destructor)) static void shutdown_libei_input(void) {
    pthread_mutex_lock(&state_lock);
    while (devices) {
        struct input_device *device = devices;
        devices = device->next;
        free(device);
    }
    if (sender_initialized) {
        eis_sender_close(&sender);
        sender_initialized = false;
        sender_connected = false;
    }
    pthread_mutex_unlock(&state_lock);
}
