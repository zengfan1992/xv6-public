#ifndef UNISTD_H__
#define UNISTD_H__

#include <stdint.h>

int getpid(void);
int dup(int);
int close(int);
int fork(void);
int chdir(const char *path);
int unlink(const char *path);
ssize_t read(int fd, void *buf, size_t count);
ssize_t write(int fd, const void *buf, size_t count);
int pipe(int fds[2]);
void *sbrk(intptr_t delta);

static inline int
execvp(const char *argv0, char *argv[])
{
	int exec(const char *argv0, char *argv[]);
	return exec(argv0, argv);
}

#endif // UNISTD_H__
