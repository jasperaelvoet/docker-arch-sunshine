#define _GNU_SOURCE

#include <errno.h>
#include <libevdev/libevdev.h>
#include <libevdev/libevdev-uinput.h>
#include <linux/input.h>
#include <stdbool.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

static void write_event(struct libevdev_uinput *device, unsigned int type, unsigned int code, int value) {
    int result = libevdev_uinput_write_event(device, type, code, value);
    if (result != 0) {
        fprintf(stderr, "arch-sunshine-libei-input-test: write_event failed: %s\n", strerror(-result));
    }
}

static void sync_device(struct libevdev_uinput *device) {
    write_event(device, EV_SYN, SYN_REPORT, 0);
}

static void tap_key(struct libevdev_uinput *device, unsigned int code) {
    write_event(device, EV_KEY, code, 1);
    sync_device(device);
    usleep(30000);
    write_event(device, EV_KEY, code, 0);
    sync_device(device);
    usleep(30000);
}

static int create_keyboard(struct libevdev_uinput **uinput) {
    struct libevdev *device = libevdev_new();
    if (!device) {
        return -ENOMEM;
    }
    libevdev_set_name(device, "Arch Sunshine Libei Keyboard Test");
    libevdev_set_id_vendor(device, 0xCAFE);
    libevdev_set_id_product(device, 0x1001);
    libevdev_enable_event_type(device, EV_KEY);
    libevdev_enable_event_code(device, EV_KEY, KEY_A, NULL);
    libevdev_enable_event_code(device, EV_KEY, KEY_B, NULL);
    libevdev_enable_event_code(device, EV_KEY, KEY_C, NULL);
    libevdev_enable_event_code(device, EV_KEY, KEY_ENTER, NULL);

    int result = libevdev_uinput_create_from_device(device, LIBEVDEV_UINPUT_OPEN_MANAGED, uinput);
    libevdev_free(device);
    return result;
}

static int create_mouse(struct libevdev_uinput **uinput) {
    struct libevdev *device = libevdev_new();
    if (!device) {
        return -ENOMEM;
    }
    libevdev_set_name(device, "Arch Sunshine Libei Mouse Test");
    libevdev_set_id_vendor(device, 0xCAFE);
    libevdev_set_id_product(device, 0x1002);
    libevdev_enable_event_type(device, EV_KEY);
    libevdev_enable_event_code(device, EV_KEY, BTN_LEFT, NULL);
    libevdev_enable_event_code(device, EV_KEY, BTN_MIDDLE, NULL);
    libevdev_enable_event_code(device, EV_KEY, BTN_RIGHT, NULL);
    libevdev_enable_event_type(device, EV_REL);
    libevdev_enable_event_code(device, EV_REL, REL_X, NULL);
    libevdev_enable_event_code(device, EV_REL, REL_Y, NULL);
    libevdev_enable_event_type(device, EV_ABS);
    struct input_absinfo x = {.minimum = 0, .maximum = 1919, .resolution = 1};
    struct input_absinfo y = {.minimum = 0, .maximum = 1079, .resolution = 1};
    libevdev_enable_event_code(device, EV_ABS, ABS_X, &x);
    libevdev_enable_event_code(device, EV_ABS, ABS_Y, &y);

    int result = libevdev_uinput_create_from_device(device, LIBEVDEV_UINPUT_OPEN_MANAGED, uinput);
    libevdev_free(device);
    return result;
}

static int run_keyboard(void) {
    struct libevdev_uinput *keyboard = NULL;
    int result = create_keyboard(&keyboard);
    if (result != 0) {
        fprintf(stderr, "arch-sunshine-libei-input-test: create keyboard failed: %s\n", strerror(-result));
        return 1;
    }
    usleep(500000);
    tap_key(keyboard, KEY_A);
    tap_key(keyboard, KEY_B);
    tap_key(keyboard, KEY_C);
    tap_key(keyboard, KEY_ENTER);
    usleep(500000);
    libevdev_uinput_destroy(keyboard);
    return 0;
}

static void click_button(struct libevdev_uinput *device, unsigned int button) {
    write_event(device, EV_KEY, button, 1);
    sync_device(device);
    usleep(50000);
    write_event(device, EV_KEY, button, 0);
    sync_device(device);
}

static int run_mouse(void) {
    struct libevdev_uinput *mouse = NULL;
    int result = create_mouse(&mouse);
    if (result != 0) {
        fprintf(stderr, "arch-sunshine-libei-input-test: create mouse failed: %s\n", strerror(-result));
        return 1;
    }
    usleep(500000);

    write_event(mouse, EV_ABS, ABS_X, 960);
    write_event(mouse, EV_ABS, ABS_Y, 540);
    sync_device(mouse);
    usleep(100000);

    const int moves[][2] = {{120, 120}, {-60, -60}, {20, 20}};
    for (size_t i = 0; i < sizeof(moves) / sizeof(moves[0]); i++) {
        write_event(mouse, EV_REL, REL_X, moves[i][0]);
        write_event(mouse, EV_REL, REL_Y, moves[i][1]);
        sync_device(mouse);
        usleep(100000);
    }
    click_button(mouse, BTN_LEFT);
    usleep(100000);
    click_button(mouse, BTN_MIDDLE);
    usleep(100000);
    click_button(mouse, BTN_RIGHT);
    usleep(100000);

    write_event(mouse, EV_KEY, BTN_MIDDLE, 1);
    sync_device(mouse);
    usleep(50000);
    write_event(mouse, EV_KEY, BTN_MIDDLE, 1);
    sync_device(mouse);
    usleep(50000);
    write_event(mouse, EV_KEY, BTN_MIDDLE, 0);
    sync_device(mouse);
    usleep(50000);
    write_event(mouse, EV_KEY, BTN_MIDDLE, 0);
    sync_device(mouse);

    usleep(500000);
    libevdev_uinput_destroy(mouse);
    return 0;
}

int main(int argc, char **argv) {
    const char *target = argc > 1 ? argv[1] : "all";
    bool keyboard = strcmp(target, "keyboard") == 0 || strcmp(target, "all") == 0;
    bool mouse = strcmp(target, "mouse") == 0 || strcmp(target, "all") == 0;
    if (!keyboard && !mouse) {
        fprintf(stderr, "usage: arch-sunshine-libei-input-test [keyboard|mouse|all]\n");
        return 2;
    }

    int status = 0;
    if (keyboard) {
        status |= run_keyboard();
    }
    if (mouse) {
        status |= run_mouse();
    }
    return status ? 1 : 0;
}
