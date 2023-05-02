#include <stddef.h>
#include <stdint.h>

static const uint32_t FILETYPE_UNUSED = 0;
static const uint32_t FILETYPE_DIR = 1;
static const uint32_t FILETYPE_FILE = 2;
static const uint32_t FILETYPE_DEV = 3;

#define DIRSIZ 24

typedef struct Dirent Dirent;
struct Dirent {
	uint64_t inum;
	uint8_t name[DIRSIZ];
};

typedef struct stat Stat;
struct stat {
    uint32_t typ;
    uint32_t dev;
    uint64_t ino;
    uint32_t nlink;
    uint64_t size;
};

static int
_mkdir(const char *path, int mode)
{
	extern int mkdir(const char *path);
	(void)mode;
	return mkdir(path);
}
#define mkdir _mkdir

int mknod(const char *name, int major, int minor);
int fstat(int fd, struct stat *buf);
