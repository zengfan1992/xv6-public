int dprintf(int fd, const char *fmt, ...);
#define printf(...) dprintf(1, __VA_ARGS__)
