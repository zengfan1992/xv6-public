#define NDIRECT 12
#define BSIZE 4096
static const size_t NOFILE = 64;
static const size_t NFILE = 1024;
static const size_t NINODE = 1024;
static const uint64_t MAXOPBLOCKS = 64;
static const size_t LOGSIZE = MAXOPBLOCKS * 8 - 1;
static const uint64_t FSSIZE = 262144;
static const size_t NINDIRECT = BSIZE / sizeof(uint64_t);
static const size_t MAXFILE = NDIRECT + NINDIRECT;

static const uint32_t FILETYPE_UNUSED = 0;
static const uint32_t FILETYPE_DIR = 1;
static const uint32_t FILETYPE_FILE = 2;
static const uint32_t FILETYPE_DEV = 3;
static const uint64_t ROOTINO = 1;

#define DIRSIZ 24

typedef struct Dirent Dirent;
struct Dirent {
	uint64_t inum;
	uint8_t name[DIRSIZ];
};

typedef struct Superblock Superblock;
struct Superblock {
	uint64_t size;		// Size of file system image in blocks
	uint64_t nblocks;	// Number of data blocks
	uint64_t ninodes;	// Number of inodes.
	uint64_t nlog;		// Number of log blocks
	uint64_t log_start;	// Block number of first log block
	uint64_t inode_start;	// Block number of first inode block
	uint64_t bmap_start;	// Block number of first free map block
};

typedef struct DInode DInode;
struct DInode {
	uint32_t typ;			// File type
	uint32_t major;			// Major device number (T_DEV only)
	uint32_t minor;			// Minor device number (T_DEV only)
	uint32_t nlink;			// Number of links to inode in file system
	uint64_t size;			// Size of file (bytes)
	uint64_t addrs[NDIRECT + 1];	// Data block addresses
};
static const size_t IPB = BSIZE / sizeof(DInode);

typedef struct Stat Stat;
struct Stat {
    uint32_t typ;
    uint32_t dev;
    uint64_t ino;
    uint32_t nlink;
    uint64_t size;
};

static inline uint64_t
iblock(const Superblock *sb, uint64_t inum)
{
	return inum / IPB + sb->inode_start;
}
