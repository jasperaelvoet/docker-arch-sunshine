#define _GNU_SOURCE

#include <ctype.h>
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <grp.h>
#include <openssl/sha.h>
#include <pwd.h>
#include <signal.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/capability.h>
#include <sys/prctl.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#define USER_DATA "/mnt/user_data"
#define CONFIG_DIR "/run/arch-sunshine/sunshine"
#define STATE_DIR USER_DATA "/var/lib/sunshine"
#define LOG_DIR USER_DATA "/var/log/arch-sunshine"
#define CONFIG_FILE CONFIG_DIR "/sunshine.conf"
#define APPS_FILE CONFIG_DIR "/apps.json"
#define CREDENTIALS_FILE CONFIG_DIR "/credentials.json"
#define API_PASSWORD_FILE CONFIG_DIR "/api-password"
#define LIBEI_INPUT_PRELOAD "/usr/local/lib/arch-sunshine-libei-input.so"
#define DYNAMIC_LINKER "/lib64/ld-linux-x86-64.so.2"

static const char *env_or_default(const char *name, const char *fallback) {
    const char *value = getenv(name);
    return value && *value ? value : fallback;
}

static bool env_flag_enabled(const char *name, bool fallback) {
    const char *value = getenv(name);
    if (!value || !*value) {
        return fallback;
    }
    return strcasecmp(value, "0") != 0 &&
           strcasecmp(value, "false") != 0 &&
           strcasecmp(value, "no") != 0 &&
           strcasecmp(value, "off") != 0;
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

static int chown_path(const char *path, uid_t uid, gid_t gid) {
    if (chown(path, uid, gid) != 0 && errno != ENOENT) {
        return -1;
    }
    return 0;
}

static int chmod_path(const char *path, mode_t mode) {
    if (chmod(path, mode) != 0 && errno != ENOENT) {
        return -1;
    }
    return 0;
}

static int grant_sunshine_capabilities(void) {
    cap_value_t caps_to_keep[] = {
        CAP_SYS_NICE,
        CAP_SYS_ADMIN,
    };
    const int cap_count = (int)(sizeof(caps_to_keep) / sizeof(caps_to_keep[0]));

    cap_t caps = cap_get_proc();
    if (!caps) {
        return -1;
    }
    if (cap_set_flag(caps, CAP_PERMITTED, cap_count, caps_to_keep, CAP_SET) != 0 ||
        cap_set_flag(caps, CAP_EFFECTIVE, cap_count, caps_to_keep, CAP_SET) != 0 ||
        cap_set_flag(caps, CAP_INHERITABLE, cap_count, caps_to_keep, CAP_SET) != 0 ||
        cap_set_proc(caps) != 0) {
        cap_free(caps);
        return -1;
    }
    cap_free(caps);

    for (int i = 0; i < cap_count; i++) {
        if (prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_RAISE, caps_to_keep[i], 0, 0) != 0) {
            return -1;
        }
    }
    return 0;
}

static int drop_to_desktop_user(bool keep_capabilities) {
    const char *user = env_or_default("SUNSHINE_DESKTOP_USER", "sunshine");
    struct passwd *pw = getpwnam(user);
    if (!pw) {
        errno = ENOENT;
        return -1;
    }
    if (keep_capabilities && prctl(PR_SET_KEEPCAPS, 1, 0, 0, 0) != 0) {
        return -1;
    }
    if (initgroups(user, pw->pw_gid) != 0) {
        return -1;
    }
    if (setgid(pw->pw_gid) != 0) {
        return -1;
    }
    if (setuid(pw->pw_uid) != 0) {
        return -1;
    }
    if (keep_capabilities && prctl(PR_SET_KEEPCAPS, 0, 0, 0, 0) != 0) {
        return -1;
    }
    return 0;
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
    const char *encoder = env_or_default("SUNSHINE_ENCODER", "vaapi");
    const char *display = env_or_default("SUNSHINE_DISPLAY", "");
    bool write_capture = *capture && strcasecmp(capture, "auto") != 0;
    bool write_encoder = *encoder && strcasecmp(encoder, "auto") != 0;

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
        "credentials_file = " CREDENTIALS_FILE "\n"
        "pkey = " STATE_DIR "/sunshine.key\n"
        "cert = " STATE_DIR "/sunshine.crt\n"
        "file_apps = " APPS_FILE "\n"
        "address_family = ipv4\n"
        "upnp = disabled\n"
        "origin_web_ui_allowed = pc\n"
        "lan_encryption_mode = 0\n"
        "wan_encryption_mode = 0\n"
        "system_tray = disabled\n"
        "global_prep_cmd = [{\"do\":\"/usr/local/bin/arch-sunshine client-start\",\"undo\":\"/usr/local/bin/arch-sunshine client-stop\"}]\n"
        "stream_audio = enabled\n"
        "audio_sink = arch_sunshine_audio\n"
        "gamepad = xone\n"
        "motion_as_ds4 = disabled\n"
        "touchpad_as_ds4 = disabled\n");

    if (write_capture) {
        fprintf(file, "capture = %s\n", capture);
    }
    if (write_encoder) {
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
    if (chmod_path(CONFIG_DIR, 0755) != 0) {
        perror("arch-sunshine-server: unlock config directory");
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
    if (chown_path(CONFIG_DIR, 0, 0) != 0 ||
        chown_path(CONFIG_FILE, 0, 0) != 0 ||
        chown_path(APPS_FILE, 0, 0) != 0 ||
        chmod_path(CONFIG_FILE, 0444) != 0 ||
        chmod_path(APPS_FILE, 0444) != 0 ||
        chmod_path(CONFIG_DIR, 0555) != 0) {
        perror("arch-sunshine-server: lock config files");
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

static int random_hex(char *target, size_t target_size) {
    if (target_size < 3 || target_size % 2 == 0) {
        errno = EINVAL;
        return -1;
    }
    size_t byte_count = (target_size - 1) / 2;
    unsigned char bytes[64];
    if (byte_count > sizeof(bytes)) {
        errno = EINVAL;
        return -1;
    }
    int random_fd = open("/dev/urandom", O_RDONLY | O_CLOEXEC);
    if (random_fd < 0) {
        return -1;
    }
    ssize_t read_bytes = read(random_fd, bytes, byte_count);
    close(random_fd);
    if (read_bytes != (ssize_t)byte_count) {
        return -1;
    }
    for (size_t i = 0; i < byte_count; i++) {
        snprintf(target + (i * 2), 3, "%02X", bytes[i]);
    }
    target[target_size - 1] = '\0';
    return 0;
}

static void sha256_hex(const char *value, char *target) {
    unsigned char digest[SHA256_DIGEST_LENGTH];
    SHA256((const unsigned char *)value, strlen(value), digest);
    for (size_t i = 0; i < sizeof(digest); i++) {
        snprintf(target + (i * 2), 3, "%02X", digest[sizeof(digest) - i - 1]);
    }
    target[SHA256_DIGEST_LENGTH * 2] = '\0';
}

static int seed_web_credentials(void) {
    char salt[33];
    char password[65];
    char password_hash[65];
    if (random_hex(salt, sizeof(salt)) != 0 || random_hex(password, sizeof(password)) != 0) {
        return -1;
    }
    char password_salt[sizeof(password) + sizeof(salt)];
    snprintf(password_salt, sizeof(password_salt), "%s%s", password, salt);
    sha256_hex(password_salt, password_hash);

    (void)chmod_path(CONFIG_DIR, 0755);
    (void)unlink(CREDENTIALS_FILE);
    (void)unlink(API_PASSWORD_FILE);

    FILE *file = fopen(CREDENTIALS_FILE, "w");
    if (!file) {
        (void)chmod_path(CONFIG_DIR, 0555);
        return -1;
    }
    fprintf(
        file,
        "{\n"
        "    \"username\": \"arch-sunshine-locked\",\n"
        "    \"salt\": \"%s\",\n"
        "    \"password\": \"%s\"\n"
        "}\n",
        salt,
        password_hash);
    int failed = ferror(file);
    failed |= fclose(file) != 0;

    FILE *password_file = fopen(API_PASSWORD_FILE, "w");
    if (!password_file) {
        failed = 1;
    } else {
        fprintf(password_file, "%s\n", password);
        failed |= ferror(password_file);
        failed |= fclose(password_file) != 0;
    }

    (void)chown_path(CREDENTIALS_FILE, 0, 0);
    (void)chown_path(API_PASSWORD_FILE, 0, 0);
    (void)chmod_path(CREDENTIALS_FILE, 0444);
    (void)chmod_path(API_PASSWORD_FILE, 0400);
    (void)chmod_path(CONFIG_DIR, 0555);
    if (failed || access(CREDENTIALS_FILE, R_OK) != 0 || access(API_PASSWORD_FILE, R_OK) != 0) {
        return -1;
    }
    return 0;
}

static int command_serve(void) {
    if (write_runtime_files() != 0) {
        return 1;
    }
    if (seed_web_credentials() != 0) {
        perror("arch-sunshine-server: seed locked web credentials");
        return 1;
    }

    pid_t sunshine_pid = fork();
    if (sunshine_pid == 0) {
        bool keep_caps = env_flag_enabled("SUNSHINE_REALTIME_CAPS", true);
        if (drop_to_desktop_user(keep_caps) != 0) {
            perror("arch-sunshine-server: drop privileges");
            _exit(127);
        }
        if (keep_caps && grant_sunshine_capabilities() != 0) {
            perror("arch-sunshine-server: grant Sunshine realtime capabilities");
        }
        setenv("ARCH_SUNSHINE_LIBEI_INPUT", "1", 1);
        execl(
            DYNAMIC_LINKER,
            "ld-linux-x86-64.so.2",
            "--preload",
            LIBEI_INPUT_PRELOAD,
            "/usr/local/bin/sunshine",
            CONFIG_FILE,
            (char *)NULL);
        perror("arch-sunshine-server: exec sunshine with libei input preload");
        _exit(127);
    }

    if (sunshine_pid < 0) {
        perror("arch-sunshine-server: fork");
        return 1;
    }

    return wait_child(sunshine_pid);
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

    const char *script =
        "import base64, json, ssl, sys, time, urllib.error, urllib.request\n"
        "pin = sys.argv[1]\n"
        "password_path = '" API_PASSWORD_FILE "'\n"
        "url = 'https://127.0.0.1:47990/api/pin'\n"
        "deadline = time.monotonic() + 30\n"
        "with open(password_path, 'r', encoding='utf-8') as f:\n"
        "    password = f.read().strip()\n"
        "auth = base64.b64encode(('arch-sunshine-locked:' + password).encode()).decode()\n"
        "payload = json.dumps({'pin': pin, 'name': 'Moonlight'}).encode()\n"
        "context = ssl._create_unverified_context()\n"
        "last_error = None\n"
        "while time.monotonic() < deadline:\n"
        "    request = urllib.request.Request(url, data=payload, method='POST', headers={\n"
        "        'Authorization': 'Basic ' + auth,\n"
        "        'Content-Type': 'application/json',\n"
        "    })\n"
        "    try:\n"
        "        with urllib.request.urlopen(request, timeout=5, context=context) as response:\n"
        "            body = response.read().decode()\n"
        "        print(body)\n"
        "        data = json.loads(body)\n"
        "        if data.get('status') is True:\n"
        "            sys.exit(0)\n"
        "        last_error = body\n"
        "    except Exception as exc:\n"
        "        last_error = str(exc)\n"
        "    time.sleep(0.25)\n"
        "print('failed to send PIN to Sunshine API: ' + str(last_error), file=sys.stderr)\n"
        "sys.exit(1)\n";

    pid_t pid = fork();
    if (pid == 0) {
        execlp("python3", "python3", "-c", script, pin, (char *)NULL);
        _exit(127);
    }
    if (pid < 0) {
        perror("arch-sunshine-server: fork");
        return 1;
    }
    int status = wait_child(pid);
    if (status == 0) {
        puts("PIN sent to Sunshine.");
    }
    return status;
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
