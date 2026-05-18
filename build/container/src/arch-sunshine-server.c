#define _GNU_SOURCE

#include <ctype.h>
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#define USER_DATA "/mnt/user_data"
#define CONFIG_DIR USER_DATA "/etc/sunshine"
#define STATE_DIR USER_DATA "/var/lib/sunshine"
#define LOG_DIR USER_DATA "/var/log/arch-sunshine"
#define PIN_FIFO STATE_DIR "/pin.fifo"
#define CONFIG_FILE CONFIG_DIR "/sunshine.conf"
#define APPS_FILE CONFIG_DIR "/apps.json"

static const char *env_or_default(const char *name, const char *fallback) {
    const char *value = getenv(name);
    return value && *value ? value : fallback;
}

static int mkdir_p(const char *path, mode_t mode) {
    char buffer[4096];
    size_t len = strlen(path);

    if (len >= sizeof(buffer)) {
        errno = ENAMETOOLONG;
        return -1;
    }

    memcpy(buffer, path, len + 1);
    for (char *p = buffer + 1; *p; p++) {
        if (*p != '/') {
            continue;
        }
        *p = '\0';
        if (mkdir(buffer, mode) != 0 && errno != EEXIST) {
            return -1;
        }
        *p = '/';
    }

    if (mkdir(buffer, mode) != 0 && errno != EEXIST) {
        return -1;
    }
    return 0;
}

static int ensure_fifo(const char *path) {
    struct stat st;

    if (lstat(path, &st) == 0) {
        if (S_ISFIFO(st.st_mode)) {
            return 0;
        }
        if (unlink(path) != 0) {
            return -1;
        }
    } else if (errno != ENOENT) {
        return -1;
    }

    return mkfifo(path, 0600);
}

static bool is_render_node(const char *name) {
    if (strncmp(name, "renderD", 7) != 0) {
        return false;
    }
    for (const char *p = name + 7; *p; p++) {
        if (!isdigit((unsigned char)*p)) {
            return false;
        }
    }
    return true;
}

static int compare_strings(const void *a, const void *b) {
    const char *const *left = a;
    const char *const *right = b;
    return strcmp(*left, *right);
}

static char *render_node(void) {
    const char *override = getenv("SUNSHINE_ADAPTER");
    if (override && *override) {
        return strdup(override);
    }

    DIR *dir = opendir("/dev/dri");
    if (!dir) {
        return strdup("");
    }

    char *nodes[128];
    size_t count = 0;
    struct dirent *entry;
    while ((entry = readdir(dir)) != NULL && count < 128) {
        if (!is_render_node(entry->d_name)) {
            continue;
        }
        if (asprintf(&nodes[count], "/dev/dri/%s", entry->d_name) >= 0) {
            count++;
        }
    }
    closedir(dir);

    if (count == 0) {
        return strdup("");
    }

    qsort(nodes, count, sizeof(nodes[0]), compare_strings);
    char *first = strdup(nodes[0]);
    for (size_t i = 0; i < count; i++) {
        free(nodes[i]);
    }
    return first ? first : strdup("");
}

static void json_string(FILE *file, const char *value) {
    fputc('"', file);
    for (const unsigned char *p = (const unsigned char *)value; *p; p++) {
        switch (*p) {
        case '\\':
            fputs("\\\\", file);
            break;
        case '"':
            fputs("\\\"", file);
            break;
        case '\b':
            fputs("\\b", file);
            break;
        case '\f':
            fputs("\\f", file);
            break;
        case '\n':
            fputs("\\n", file);
            break;
        case '\r':
            fputs("\\r", file);
            break;
        case '\t':
            fputs("\\t", file);
            break;
        default:
            if (*p < 0x20) {
                fprintf(file, "\\u%04x", *p);
            } else {
                fputc(*p, file);
            }
        }
    }
    fputc('"', file);
}

static void json_env_line(FILE *file, const char *key, const char *value, bool comma) {
    fputs("    ", file);
    json_string(file, key);
    fputs(": ", file);
    json_string(file, value);
    fputs(comma ? ",\n" : "\n", file);
}

static int write_config(void) {
    char *adapter = render_node();
    const char *capture = env_or_default("SUNSHINE_CAPTURE", "kwin");
    const char *encoder = env_or_default("SUNSHINE_ENCODER", "");
    const char *display = env_or_default("SUNSHINE_DISPLAY", "");
    bool write_capture = *capture && strcasecmp(capture, "auto") != 0;

    FILE *file = fopen(CONFIG_FILE, "w");
    if (!file) {
        free(adapter);
        return -1;
    }

    fprintf(
        file,
        "sunshine_name = Docker Arch Sunshine\n"
        "min_log_level = info\n"
        "log_path = " LOG_DIR "/sunshine.log\n"
        "file_state = " STATE_DIR "/sunshine_state.json\n"
        "credentials_file = " STATE_DIR "/sunshine_state.json\n"
        "pkey = " STATE_DIR "/sunshine.key\n"
        "cert = " STATE_DIR "/sunshine.crt\n"
        "file_apps = " APPS_FILE "\n"
        "address_family = ipv4\n"
        "upnp = disabled\n"
        "origin_web_ui_allowed = wan\n"
        "lan_encryption_mode = 0\n"
        "wan_encryption_mode = 0\n"
        "system_tray = disabled\n"
        "global_prep_cmd = [{\"do\":\"/usr/local/bin/arch-sunshine client-start\",\"undo\":\"/usr/local/bin/arch-sunshine client-stop\"}]\n"
        "stream_audio = enabled\n"
        "audio_sink = arch_sunshine_audio.monitor\n"
        "virtual_sink = arch_sunshine_audio\n"
        "gamepad = x360\n");

    if (write_capture) {
        fprintf(file, "capture = %s\n", capture);
    }
    if (*encoder) {
        fprintf(file, "encoder = %s\n", encoder);
    }
    if (*display) {
        fprintf(file, "display_name = %s\n", display);
    }
    if (adapter && *adapter) {
        fprintf(file, "adapter_name = %s\n", adapter);
    }

    int failed = ferror(file);
    failed |= fclose(file) != 0;
    free(adapter);
    return failed ? -1 : 0;
}

static int write_apps(void) {
    const char *runtime_dir = env_or_default("XDG_RUNTIME_DIR", "/run/user/1000");
    const char *wayland_display = env_or_default("WAYLAND_DISPLAY", "arch-sunshine-wayland");
    const char *display = env_or_default("DISPLAY", env_or_default("SUNSHINE_XWAYLAND_DISPLAY", ":0"));
    const char *pulse_server = env_or_default("PULSE_SERVER", "unix:/run/user/1000/pulse/native");

    FILE *file = fopen(APPS_FILE, "w");
    if (!file) {
        return -1;
    }

    fputs("{\n  \"env\": {\n", file);
    json_env_line(file, "PATH", "$(PATH):$(HOME)/.local/bin", true);
    json_env_line(file, "LANG", "en_US.UTF-8", true);
    json_env_line(file, "LC_CTYPE", "en_US.UTF-8", true);
    json_env_line(file, "XDG_RUNTIME_DIR", runtime_dir, true);
    json_env_line(file, "WAYLAND_DISPLAY", wayland_display, true);
    json_env_line(file, "DISPLAY", display, true);
    json_env_line(file, "XDG_SESSION_TYPE", "wayland", true);
    json_env_line(file, "XDG_CURRENT_DESKTOP", "KDE", true);
    json_env_line(file, "XDG_SESSION_DESKTOP", "KDE", true);
    json_env_line(file, "DESKTOP_SESSION", "plasma", true);
    json_env_line(file, "KDE_SESSION_VERSION", "6", true);
    json_env_line(file, "KDE_FULL_SESSION", "true", true);
    json_env_line(file, "QT_QPA_PLATFORM", "wayland", true);
    json_env_line(file, "SDL_VIDEODRIVER", "wayland,x11", true);
    json_env_line(file, "MOZ_ENABLE_WAYLAND", "1", true);
    json_env_line(file, "GIO_USE_NETWORK_MONITOR", "base", true);
    json_env_line(file, "RES_OPTIONS", "timeout:1 attempts:2 rotate single-request-reopen", true);
    json_env_line(file, "PULSE_SERVER", pulse_server, true);
    json_env_line(file, "STEAM_RUNTIME", "1", true);
    json_env_line(file, "SRT_URLOPEN_PREFER_STEAM", "1", true);
    json_env_line(file, "STEAM_DISABLE_AUDIO_DEVICE_SWITCHING", "1", false);
    fputs("  },\n  \"apps\": [\n    {\n      \"name\": \"Desktop\",\n      \"image-path\": \"desktop.png\"\n    }\n  ]\n}\n", file);

    int failed = ferror(file);
    failed |= fclose(file) != 0;
    return failed ? -1 : 0;
}

static int write_runtime_files(void) {
    if (mkdir_p(CONFIG_DIR, 0755) != 0 || mkdir_p(STATE_DIR, 0755) != 0 || mkdir_p(LOG_DIR, 0755) != 0) {
        perror("arch-sunshine-server: mkdir");
        return 1;
    }
    if (write_config() != 0) {
        perror("arch-sunshine-server: write sunshine.conf");
        return 1;
    }
    if (write_apps() != 0) {
        perror("arch-sunshine-server: write apps.json");
        return 1;
    }
    if (ensure_fifo(PIN_FIFO) != 0) {
        perror("arch-sunshine-server: create pin fifo");
        return 1;
    }
    return 0;
}

static int wait_child(pid_t pid) {
    int status = 0;
    while (waitpid(pid, &status, 0) < 0) {
        if (errno != EINTR) {
            return 1;
        }
    }
    if (WIFEXITED(status)) {
        return WEXITSTATUS(status);
    }
    if (WIFSIGNALED(status)) {
        return 128 + WTERMSIG(status);
    }
    return 1;
}

static void seed_web_credentials(void) {
    pid_t pid = fork();
    if (pid == 0) {
        int null_fd = open("/dev/null", O_RDWR);
        if (null_fd >= 0) {
            dup2(null_fd, STDOUT_FILENO);
            dup2(null_fd, STDERR_FILENO);
            close(null_fd);
        }
        execl(
            "/usr/local/bin/sunshine",
            "sunshine",
            CONFIG_FILE,
            "--creds",
            env_or_default("SUNSHINE_WEB_UI_USER", "sunshine"),
            env_or_default("SUNSHINE_WEB_UI_PASS", "sunshine"),
            (char *)NULL);
        _exit(127);
    }
    if (pid > 0) {
        (void)wait_child(pid);
    }
}

static void fifo_pump(int write_fd) {
    for (;;) {
        int read_fd = open(PIN_FIFO, O_RDONLY);
        if (read_fd < 0) {
            usleep(100000);
            continue;
        }

        char buffer[256];
        ssize_t bytes;
        while ((bytes = read(read_fd, buffer, sizeof(buffer))) > 0) {
            char *cursor = buffer;
            ssize_t remaining = bytes;
            while (remaining > 0) {
                ssize_t written = write(write_fd, cursor, (size_t)remaining);
                if (written < 0) {
                    close(read_fd);
                    _exit(errno == EPIPE ? 0 : 1);
                }
                cursor += written;
                remaining -= written;
            }
        }
        close(read_fd);
    }
}

static int command_serve(void) {
    if (write_runtime_files() != 0) {
        return 1;
    }
    seed_web_credentials();

    int pipe_fds[2];
    if (pipe(pipe_fds) != 0) {
        perror("arch-sunshine-server: pipe");
        return 1;
    }

    pid_t pump_pid = fork();
    if (pump_pid == 0) {
        close(pipe_fds[0]);
        fifo_pump(pipe_fds[1]);
        _exit(0);
    }
    if (pump_pid < 0) {
        perror("arch-sunshine-server: fork");
        close(pipe_fds[0]);
        close(pipe_fds[1]);
        return 1;
    }

    pid_t sunshine_pid = fork();
    if (sunshine_pid == 0) {
        close(pipe_fds[1]);
        dup2(pipe_fds[0], STDIN_FILENO);
        close(pipe_fds[0]);
        execl("/usr/local/bin/sunshine", "sunshine", "-0", CONFIG_FILE, (char *)NULL);
        _exit(127);
    }
    close(pipe_fds[0]);
    close(pipe_fds[1]);

    if (sunshine_pid < 0) {
        perror("arch-sunshine-server: fork");
        kill(pump_pid, SIGTERM);
        (void)wait_child(pump_pid);
        return 1;
    }

    int status = wait_child(sunshine_pid);
    kill(pump_pid, SIGTERM);
    (void)wait_child(pump_pid);
    return status;
}

static int command_pin(const char *pin) {
    if (!pin || strlen(pin) != 4) {
        fprintf(stderr, "PIN must be exactly 4 digits.\n");
        return 2;
    }
    for (const char *p = pin; *p; p++) {
        if (!isdigit((unsigned char)*p)) {
            fprintf(stderr, "PIN must be exactly 4 digits.\n");
            return 2;
        }
    }

    struct timespec start;
    clock_gettime(CLOCK_MONOTONIC, &start);
    int fd = -1;
    for (;;) {
        fd = open(PIN_FIFO, O_WRONLY | O_NONBLOCK);
        if (fd >= 0) {
            break;
        }
        if (errno != ENOENT && errno != ENXIO) {
            perror("arch-sunshine-server: open pin fifo");
            return 1;
        }

        struct timespec now;
        clock_gettime(CLOCK_MONOTONIC, &now);
        if ((now.tv_sec - start.tv_sec) >= 30) {
            fprintf(stderr, "Sunshine PIN FIFO is not ready: %s\n", PIN_FIFO);
            return 1;
        }
        usleep(100000);
    }

    dprintf(fd, "%s\n", pin);
    close(fd);
    puts("PIN sent to Sunshine.");
    return 0;
}

int main(int argc, char **argv) {
    if (argc == 2 && strcmp(argv[1], "serve") == 0) {
        return command_serve();
    }
    if (argc == 3 && strcmp(argv[1], "pin") == 0) {
        return command_pin(argv[2]);
    }

    fprintf(stderr, "usage: arch-sunshine-server serve | pin PIN\n");
    return 2;
}
