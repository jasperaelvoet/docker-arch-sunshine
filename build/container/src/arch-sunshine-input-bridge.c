#define _GNU_SOURCE

#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <gio/gio.h>
#include <gio/gunixfdlist.h>
#include <libei.h>
#include <linux/input.h>
#include <limits.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/select.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <time.h>
#include <unistd.h>

#define MAX_EI_DEVICES 64
#define MAX_INPUT_DEVICES 64
#define KEY_STATE_BYTES ((KEY_MAX + 8) / 8)
#define PORTAL_DEVICE_KEYBOARD (1 << 0)
#define PORTAL_DEVICE_POINTER (1 << 1)
#define PORTAL_DEVICE_TOUCHSCREEN (1 << 2)
#define PORTAL_DEVICE_ALL (PORTAL_DEVICE_KEYBOARD | PORTAL_DEVICE_POINTER | PORTAL_DEVICE_TOUCHSCREEN)

struct input_device {
    char path[PATH_MAX];
    char name[256];
    int fd;
    bool swap_primary_buttons;
    int32_t rel_x;
    int32_t rel_y;
    int32_t scroll_x;
    int32_t scroll_y;
    bool have_abs_x;
    bool have_abs_y;
    int32_t abs_x;
    int32_t abs_y;
    int32_t abs_min_x;
    int32_t abs_max_x;
    int32_t abs_min_y;
    int32_t abs_max_y;
    uint8_t key_state[KEY_STATE_BYTES];
};

struct input_devices {
    struct input_device items[MAX_INPUT_DEVICES];
    size_t count;
    double last_scan;
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

static volatile sig_atomic_t stopping = 0;

static void log_message(const char *message) {
    fprintf(stdout, "arch-sunshine-input-bridge: %s\n", message);
    fflush(stdout);
}

static double monotonic_seconds(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (double)ts.tv_sec + (double)ts.tv_nsec / 1000000000.0;
}

static void stop_handler(int signum) {
    (void)signum;
    stopping = 1;
}

static bool starts_with(const char *text, const char *prefix) {
    return strncmp(text, prefix, strlen(prefix)) == 0;
}

static bool wanted_device(const char *name) {
    return starts_with(name, "Mouse passthrough") ||
           starts_with(name, "Keyboard passthrough") ||
           starts_with(name, "Arch Sunshine Input Test");
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

static bool should_swap_primary_buttons(const char *name) {
    return starts_with(name, "Mouse passthrough") &&
           env_truthy("SUNSHINE_SWAP_PRIMARY_MOUSE_BUTTONS", "1");
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
    } else {
        *state &= (uint8_t)~mask;
    }
    return true;
}

static bool read_input_name(const char *event_name, char *name, size_t name_size) {
    char path[PATH_MAX];
    snprintf(path, sizeof(path), "/sys/class/input/%s/device/name", event_name);

    FILE *file = fopen(path, "r");
    if (!file) {
        name[0] = '\0';
        return false;
    }
    bool ok = fgets(name, (int)name_size, file) != NULL;
    fclose(file);

    if (!ok) {
        name[0] = '\0';
        return false;
    }
    name[strcspn(name, "\r\n")] = '\0';
    return true;
}

static void read_abs_info(int fd, int code, int32_t *minimum, int32_t *maximum) {
    struct input_absinfo info;
    if (ioctl(fd, EVIOCGABS(code), &info) == 0) {
        *minimum = info.minimum;
        *maximum = info.maximum;
        return;
    }
    *minimum = 0;
    *maximum = 65535;
}

static void input_device_close(struct input_device *device) {
    if (device->fd >= 0) {
        close(device->fd);
        device->fd = -1;
    }
}

static bool input_device_open(struct input_device *device, const char *path, const char *name) {
    memset(device, 0, sizeof(*device));
    snprintf(device->path, sizeof(device->path), "%s", path);
    snprintf(device->name, sizeof(device->name), "%s", name);
    device->fd = open(path, O_RDONLY | O_NONBLOCK | O_CLOEXEC);
    if (device->fd < 0) {
        return false;
    }
    device->swap_primary_buttons = should_swap_primary_buttons(name);
    read_abs_info(device->fd, ABS_X, &device->abs_min_x, &device->abs_max_x);
    read_abs_info(device->fd, ABS_Y, &device->abs_min_y, &device->abs_max_y);
    return true;
}

static ssize_t find_input_device(struct input_devices *devices, const char *path) {
    for (size_t i = 0; i < devices->count; i++) {
        if (strcmp(devices->items[i].path, path) == 0) {
            return (ssize_t)i;
        }
    }
    return -1;
}

static int compare_dirent_names(const void *a, const void *b) {
    const struct dirent *const *left = a;
    const struct dirent *const *right = b;
    return strcmp((*left)->d_name, (*right)->d_name);
}

static bool is_event_node(const char *name) {
    if (!starts_with(name, "event")) {
        return false;
    }
    for (const char *p = name + 5; *p; p++) {
        if (*p < '0' || *p > '9') {
            return false;
        }
    }
    return true;
}

static void input_devices_scan(struct input_devices *devices, bool force) {
    double now = monotonic_seconds();
    if (!force && now - devices->last_scan < 0.5) {
        return;
    }
    devices->last_scan = now;

    bool seen[MAX_INPUT_DEVICES] = {0};
    DIR *dir = opendir("/dev/input");
    if (!dir) {
        return;
    }

    struct dirent *entries[256];
    size_t entry_count = 0;
    struct dirent *entry;
    while ((entry = readdir(dir)) != NULL && entry_count < 256) {
        if (!is_event_node(entry->d_name)) {
            continue;
        }
        struct dirent *copy = malloc(sizeof(*copy));
        if (!copy) {
            continue;
        }
        memcpy(copy, entry, sizeof(*copy));
        entries[entry_count++] = copy;
    }
    closedir(dir);
    qsort(entries, entry_count, sizeof(entries[0]), compare_dirent_names);

    for (size_t i = 0; i < entry_count; i++) {
        char name[256];
        char path[PATH_MAX];
        snprintf(path, sizeof(path), "/dev/input/%s", entries[i]->d_name);
        if (!read_input_name(entries[i]->d_name, name, sizeof(name)) || !wanted_device(name)) {
            free(entries[i]);
            continue;
        }

        ssize_t index = find_input_device(devices, path);
        if (index >= 0) {
            seen[index] = true;
            free(entries[i]);
            continue;
        }
        if (devices->count >= MAX_INPUT_DEVICES) {
            free(entries[i]);
            continue;
        }

        struct input_device *device = &devices->items[devices->count];
        if (input_device_open(device, path, name)) {
            printf(
                "arch-sunshine-input-bridge: watching %s (%s)%s\n",
                path,
                name,
                device->swap_primary_buttons ? " primary buttons swapped" : "");
            fflush(stdout);
            seen[devices->count] = true;
            devices->count++;
        } else {
            printf("arch-sunshine-input-bridge: could not open %s: %s\n", path, strerror(errno));
            fflush(stdout);
        }
        free(entries[i]);
    }

    for (ssize_t i = (ssize_t)devices->count - 1; i >= 0; i--) {
        if (seen[i] && access(devices->items[i].path, F_OK) == 0) {
            continue;
        }
        input_device_close(&devices->items[i]);
        if ((size_t)i + 1 < devices->count) {
            memmove(&devices->items[i], &devices->items[i + 1], (devices->count - (size_t)i - 1) * sizeof(devices->items[0]));
        }
        devices->count--;
    }
}

static void input_devices_close(struct input_devices *devices) {
    for (size_t i = 0; i < devices->count; i++) {
        input_device_close(&devices->items[i]);
    }
    devices->count = 0;
}

static void eis_sender_init(struct eis_sender *sender) {
    memset(sender, 0, sizeof(*sender));
    sender->fd = -1;
    sender->sequence = 1;
    sender->region_width = 1920;
    sender->region_height = 1080;
}

static void eis_sender_classify_device(struct eis_sender *sender, struct ei_device *device) {
    if (ei_device_has_capability(device, EI_DEVICE_CAP_POINTER)) {
        sender->pointer = device;
    }
    if (ei_device_has_capability(device, EI_DEVICE_CAP_POINTER_ABSOLUTE)) {
        sender->pointer_abs = device;
        struct ei_region *region = ei_device_get_region(device, 0);
        if (region) {
            sender->region_x = ei_region_get_x(region);
            sender->region_y = ei_region_get_y(region);
            sender->region_width = ei_region_get_width(region);
            sender->region_height = ei_region_get_height(region);
        }
    }
    if (ei_device_has_capability(device, EI_DEVICE_CAP_BUTTON)) {
        sender->button = device;
    }
    if (ei_device_has_capability(device, EI_DEVICE_CAP_SCROLL)) {
        sender->scroll = device;
    }
    if (ei_device_has_capability(device, EI_DEVICE_CAP_KEYBOARD)) {
        sender->keyboard = device;
    }
}

static void eis_sender_classify_all(struct eis_sender *sender) {
    sender->pointer = NULL;
    sender->pointer_abs = NULL;
    sender->button = NULL;
    sender->scroll = NULL;
    sender->keyboard = NULL;
    for (size_t i = 0; i < sender->device_count; i++) {
        eis_sender_classify_device(sender, sender->devices[i]);
    }
}

static void eis_sender_remove_device(struct eis_sender *sender, struct ei_device *device) {
    for (size_t i = 0; i < sender->device_count; i++) {
        if (sender->devices[i] != device) {
            continue;
        }
        ei_device_unref(sender->devices[i]);
        if (i + 1 < sender->device_count) {
            memmove(&sender->devices[i], &sender->devices[i + 1], (sender->device_count - i - 1) * sizeof(sender->devices[0]));
        }
        sender->device_count--;
        break;
    }
    eis_sender_classify_all(sender);
}

static void eis_sender_drain_events(struct eis_sender *sender) {
    struct ei_event *event;
    while ((event = ei_get_event(sender->ei)) != NULL) {
        enum ei_event_type type = ei_event_get_type(event);
        switch (type) {
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
            if (device && sender->device_count < MAX_EI_DEVICES) {
                sender->devices[sender->device_count++] = ei_device_ref(device);
                eis_sender_classify_device(sender, device);
            }
            break;
        }
        case EI_EVENT_DEVICE_RESUMED: {
            struct ei_device *device = ei_event_get_device(event);
            if (device) {
                ei_device_start_emulating(device, sender->sequence);
                sender->sequence++;
                if (sender->sequence == 0) {
                    sender->sequence = 1;
                }
            }
            break;
        }
        case EI_EVENT_DEVICE_REMOVED:
            eis_sender_remove_device(sender, ei_event_get_device(event));
            break;
        default:
            break;
        }
        ei_event_unref(event);
    }
}

static void eis_sender_pump(struct eis_sender *sender, double timeout) {
    double deadline = monotonic_seconds() + timeout;
    for (;;) {
        double wait = deadline - monotonic_seconds();
        if (wait < 0) {
            wait = 0;
        }
        if (wait > 0.05) {
            wait = 0.05;
        }

        if (sender->fd >= 0) {
            fd_set read_fds;
            FD_ZERO(&read_fds);
            FD_SET(sender->fd, &read_fds);
            struct timeval tv = {
                .tv_sec = (time_t)wait,
                .tv_usec = (suseconds_t)((wait - (time_t)wait) * 1000000.0),
            };
            int ready = select(sender->fd + 1, &read_fds, NULL, NULL, &tv);
            if (ready > 0 && FD_ISSET(sender->fd, &read_fds)) {
                ei_dispatch(sender->ei);
            }
        }

        eis_sender_drain_events(sender);
        if (monotonic_seconds() >= deadline) {
            break;
        }
    }
}

static int connect_to_eis_fd(struct eis_sender *sender, GError **error) {
    sender->bus = g_bus_get_sync(G_BUS_TYPE_SESSION, NULL, error);
    if (!sender->bus) {
        return -1;
    }

    GUnixFDList *fd_list = NULL;
    GVariant *reply = g_dbus_connection_call_with_unix_fd_list_sync(
        sender->bus,
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
        sender->cookie = g_variant_ref_sink(cookie_value);
    }
    g_variant_unref(cookie_value);
    g_variant_unref(reply);
    g_object_unref(fd_list);
    return fd;
}

static bool eis_sender_connect(struct eis_sender *sender, GError **error) {
    int fd = connect_to_eis_fd(sender, error);
    if (fd < 0) {
        return false;
    }

    sender->ei = ei_new_sender(NULL);
    if (!sender->ei) {
        close(fd);
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_FAILED, "failed to create libei sender");
        return false;
    }

    ei_configure_name(sender->ei, "arch-sunshine-input-bridge");
    int result = ei_setup_backend_fd(sender->ei, fd);
    if (result != 0) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_FAILED, "ei_setup_backend_fd failed: %s", strerror(-result));
        return false;
    }
    sender->fd = ei_get_fd(sender->ei);
    eis_sender_pump(sender, 3.0);
    if (!sender->keyboard && !sender->pointer && !sender->pointer_abs && !sender->button) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_FAILED, "KWin EIS did not expose any input devices");
        return false;
    }
    return true;
}

static void eis_sender_close(struct eis_sender *sender) {
    if (sender->bus && sender->cookie) {
        GVariant *children[] = {sender->cookie};
        GError *error = NULL;
        GVariant *reply = g_dbus_connection_call_sync(
            sender->bus,
            "org.kde.KWin",
            "/org/kde/KWin/EIS/RemoteDesktop",
            "org.kde.KWin.EIS.RemoteDesktop",
            "disconnect",
            g_variant_new_tuple(children, 1),
            NULL,
            G_DBUS_CALL_FLAGS_NONE,
            -1,
            NULL,
            &error);
        if (reply) {
            g_variant_unref(reply);
        }
        if (error) {
            g_error_free(error);
        }
    }
    for (size_t i = 0; i < sender->device_count; i++) {
        ei_device_unref(sender->devices[i]);
    }
    if (sender->ei) {
        ei_unref(sender->ei);
    }
    if (sender->cookie) {
        g_variant_unref(sender->cookie);
    }
    if (sender->bus) {
        g_object_unref(sender->bus);
    }
    eis_sender_init(sender);
}

static void eis_frame(struct eis_sender *sender, struct ei_device *device) {
    if (!device) {
        return;
    }
    ei_device_frame(device, ei_now(sender->ei));
    ei_dispatch(sender->ei);
    eis_sender_pump(sender, 0.0);
}

static void eis_move_relative(struct eis_sender *sender, int32_t dx, int32_t dy) {
    if (sender->pointer && (dx || dy)) {
        ei_device_pointer_motion(sender->pointer, (double)dx, (double)dy);
        eis_frame(sender, sender->pointer);
    }
}

static void eis_move_absolute(struct eis_sender *sender, struct input_device *device) {
    if (!sender->pointer_abs || !device->have_abs_x || !device->have_abs_y) {
        return;
    }
    if (device->abs_max_x <= device->abs_min_x ||
        device->abs_max_y <= device->abs_min_y ||
        sender->region_width == 0 ||
        sender->region_height == 0) {
        return;
    }

    double target_x = (double)sender->region_x +
                      ((double)(device->abs_x - device->abs_min_x) /
                       (double)(device->abs_max_x - device->abs_min_x)) *
                          (double)(sender->region_width - 1);
    double target_y = (double)sender->region_y +
                      ((double)(device->abs_y - device->abs_min_y) /
                       (double)(device->abs_max_y - device->abs_min_y)) *
                          (double)(sender->region_height - 1);
    ei_device_pointer_motion_absolute(sender->pointer_abs, target_x, target_y);
    eis_frame(sender, sender->pointer_abs);
}

static void eis_button(struct eis_sender *sender, uint32_t code, bool pressed) {
    if (sender->button) {
        ei_device_button_button(sender->button, code, pressed);
        eis_frame(sender, sender->button);
    }
}

static void eis_scroll(struct eis_sender *sender, int32_t dx, int32_t dy) {
    if (sender->scroll && (dx || dy)) {
        ei_device_scroll_discrete(sender->scroll, dx, dy);
        eis_frame(sender, sender->scroll);
    }
}

static void eis_key(struct eis_sender *sender, uint32_t code, bool pressed) {
    if (sender->keyboard) {
        ei_device_keyboard_key(sender->keyboard, code, pressed);
        eis_frame(sender, sender->keyboard);
    }
}

static void flush_device(struct input_device *device, struct eis_sender *sender) {
    if (device->rel_x || device->rel_y) {
        eis_move_relative(sender, device->rel_x, device->rel_y);
        device->rel_x = 0;
        device->rel_y = 0;
    }
    eis_move_absolute(sender, device);
    if (device->scroll_x || device->scroll_y) {
        eis_scroll(sender, device->scroll_x, device->scroll_y);
        device->scroll_x = 0;
        device->scroll_y = 0;
    }
}

static void handle_event(struct input_device *device, const struct input_event *event, struct eis_sender *sender) {
    if (event->type == EV_REL) {
        if (event->code == REL_X) {
            device->rel_x += event->value;
        } else if (event->code == REL_Y) {
            device->rel_y += event->value;
        } else if (event->code == REL_HWHEEL_HI_RES) {
            device->scroll_x += event->value;
        } else if (event->code == REL_WHEEL_HI_RES) {
            device->scroll_y -= event->value;
        } else if (event->code == REL_HWHEEL) {
            device->scroll_x += event->value * 120;
        } else if (event->code == REL_WHEEL) {
            device->scroll_y -= event->value * 120;
        }
    } else if (event->type == EV_ABS) {
        if (event->code == ABS_X) {
            device->abs_x = event->value;
            device->have_abs_x = true;
        } else if (event->code == ABS_Y) {
            device->abs_y = event->value;
            device->have_abs_y = true;
        }
    } else if (event->type == EV_KEY) {
        if (event->value != 0 && event->value != 1) {
            return;
        }
        uint32_t code = event->code;
        bool pressed = event->value == 1;
        if (!key_state_changed(device, code, pressed)) {
            return;
        }
        if (code >= BTN_MOUSE && code < BTN_JOYSTICK) {
            if (device->swap_primary_buttons) {
                if (code == BTN_LEFT) {
                    code = BTN_RIGHT;
                } else if (code == BTN_RIGHT) {
                    code = BTN_LEFT;
                }
            }
            eis_button(sender, code, pressed);
        } else {
            eis_key(sender, code, pressed);
        }
    } else if (event->type == EV_SYN && event->code == SYN_REPORT) {
        flush_device(device, sender);
    }
}

static bool read_events(struct input_device *device, struct eis_sender *sender) {
    struct input_event events[64];
    for (;;) {
        ssize_t bytes = read(device->fd, events, sizeof(events));
        if (bytes < 0) {
            if (errno == EAGAIN || errno == EWOULDBLOCK) {
                return true;
            }
            return !(errno == ENODEV || errno == EIO);
        }
        if (bytes == 0) {
            return true;
        }
        size_t count = (size_t)bytes / sizeof(events[0]);
        for (size_t i = 0; i < count; i++) {
            handle_event(device, &events[i], sender);
        }
    }
}

static int run_bridge_once(void) {
    struct eis_sender sender;
    struct input_devices devices = {0};
    eis_sender_init(&sender);

    GError *error = NULL;
    if (!eis_sender_connect(&sender, &error)) {
        if (error) {
            log_message(error->message);
            g_error_free(error);
        } else {
            log_message("failed to connect to KWin EIS");
        }
        eis_sender_close(&sender);
        return 1;
    }

    log_message("connected to KWin EIS");
    input_devices_scan(&devices, true);

    while (!stopping) {
        eis_sender_pump(&sender, 0.0);
        input_devices_scan(&devices, false);

        fd_set read_fds;
        FD_ZERO(&read_fds);
        int max_fd = -1;
        for (size_t i = 0; i < devices.count; i++) {
            FD_SET(devices.items[i].fd, &read_fds);
            if (devices.items[i].fd > max_fd) {
                max_fd = devices.items[i].fd;
            }
        }

        if (max_fd < 0) {
            usleep(100000);
            continue;
        }

        struct timeval tv = {.tv_sec = 0, .tv_usec = 100000};
        int ready = select(max_fd + 1, &read_fds, NULL, NULL, &tv);
        if (ready < 0) {
            if (errno == EINTR) {
                continue;
            }
            break;
        }
        for (size_t i = 0; i < devices.count; i++) {
            if (FD_ISSET(devices.items[i].fd, &read_fds) && !read_events(&devices.items[i], &sender)) {
                input_devices_scan(&devices, true);
                break;
            }
        }
    }

    input_devices_close(&devices);
    eis_sender_close(&sender);
    return 0;
}

int main(int argc, char **argv) {
    bool exit_on_error = false;
    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "--exit-on-error") == 0) {
            exit_on_error = true;
        } else {
            fprintf(stderr, "usage: arch-sunshine-input-bridge [--exit-on-error]\n");
            return 2;
        }
    }

    signal(SIGTERM, stop_handler);
    signal(SIGINT, stop_handler);

    while (!stopping) {
        int status = run_bridge_once();
        if (status == 0 || exit_on_error) {
            return status;
        }
        sleep(1);
    }
    return 0;
}
