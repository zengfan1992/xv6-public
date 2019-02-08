void *memset(void *p, int b, size_t n);
void *memcpy(void *d, const void *s, size_t n);
void *memmove(void *d, const void *s, size_t n);

size_t strlcpy(char *dst, const char *restrict src, size_t size);
size_t strlcat(char *dst, const char *restrict src, size_t size);
size_t strlen(const char *s);
int strcmp(const char *a, const char *b);
char *strchr(const char *s, int c);
