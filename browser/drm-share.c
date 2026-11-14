#define _GNU_SOURCE
#include <fcntl.h>
#include <stdarg.h>
#include <stdlib.h>
#include <string.h>
#include <sys/syscall.h>
#include <unistd.h>

static int inherited_drm_fd(void) {
    const char *value = getenv("TV_DRM_SHARED_FD");
    if (!value || !*value) return -1;
    char *end = NULL;
    long fd = strtol(value, &end, 10);
    return end && !*end && fd >= 0 && fd < 4096 ? (int)fd : -1;
}

static int is_card(const char *path) {
    return path && strstr(path, "/dev/dri/card") != NULL;
}

static int open_shared_card(const char *path) {
    int master = inherited_drm_fd();
    if (master < 0 || !is_card(path)) return -1;
    return (int)syscall(SYS_fcntl, master, F_DUPFD_CLOEXEC, 3);
}

int open(const char *path, int flags, ...) {
    int fd = open_shared_card(path);
    if (fd >= 0 || (inherited_drm_fd() >= 0 && is_card(path))) return fd;
    mode_t mode = 0;
    if (flags & O_CREAT) {
        va_list ap; va_start(ap, flags); mode = (mode_t)va_arg(ap, int); va_end(ap);
    }
    return (int)syscall(SYS_openat, AT_FDCWD, path, flags, mode);
}

int openat(int dirfd, const char *path, int flags, ...) {
    if (path && path[0] == '/') {
        int fd = open_shared_card(path);
        if (fd >= 0 || (inherited_drm_fd() >= 0 && is_card(path))) return fd;
    }
    mode_t mode = 0;
    if (flags & O_CREAT) {
        va_list ap; va_start(ap, flags); mode = (mode_t)va_arg(ap, int); va_end(ap);
    }
    return (int)syscall(SYS_openat, dirfd, path, flags, mode);
}

/* Cog shares the launcher's DRM file, so it must not drop master on exit. */
int drmSetMaster(int fd) { (void)fd; return inherited_drm_fd() >= 0 ? 0 : -1; }
int drmDropMaster(int fd) { (void)fd; return inherited_drm_fd() >= 0 ? 0 : -1; }
