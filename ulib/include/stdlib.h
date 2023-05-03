int atoi(const char *s);
void *malloc(size_t n);
void free(void *p);

static inline void
_exit(int s)
{
	void exit(void);

	(void)s;
	exit();
}
#define exit(s) _exit(s)
