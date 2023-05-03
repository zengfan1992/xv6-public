#include <assert.h>
#include <fcntl.h>
#include <stddef.h>
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "rxv64.h"

// Disk layout:
// [ boot block | sb block | log | inode blocks | free bit map | data blocks ]

size_t nbitmap = FSSIZE/(BSIZE*8) + 1;
size_t ninodeblocks = NINODE / IPB + 1;
size_t nlog = LOGSIZE;
size_t nmeta;    // Number of meta blocks (boot, sb, nlog, inode, bitmap)
size_t nblocks;  // Number of data blocks

int fsfd;
Superblock sb;
uint8_t zeroes[BSIZE];
size_t freeinode = 1;
size_t freeblock;
int logsec=0;


void balloc(size_t);
void wsect(uint64_t, void *);
void winode(uint64_t, DInode *);
void rinode(uint64_t inum, DInode *ip);
void rsect(uint64_t sec, void *buf);
uint64_t ialloc(uint32_t typ);
void iappend(uint64_t inum, void *p, size_t n);

// convert to intel byte order
uint32_t
xuint32(uint32_t x)
{
  unsigned char buf[4];

  buf[0] = x & 0xFF;
  buf[1] = (x >> 8) & 0xFF;
  buf[2] = (x >> 16) & 0xFF;
  buf[3] = (x >> 24) & 0xFF;
  memcpy(&x, buf, sizeof(buf));

  return x;
}

uint64_t
xuint64(uint64_t x)
{
  unsigned char buf[8];

  buf[0] = x & 0xFF;
  buf[1] = (x >> 8) & 0xFF;
  buf[2] = (x >> 16) & 0xFF;
  buf[3] = (x >> 24) & 0xFF;
  buf[4] = (x >> 32) & 0xFF;
  buf[5] = (x >> 40) & 0xFF;
  buf[6] = (x >> 48) & 0xFF;
  buf[7] = (x >> 56) & 0xFF;
  memcpy(&x, buf, sizeof(buf));

  return x;
}

int
main(int argc, char *argv[])
{
  int i, cc, fd;
  uint64_t rootino, inum, off;
  Dirent de;
  char buf[BSIZE];
  DInode din;

  if(argc < 2){
    fprintf(stderr, "Usage: mkfs fs.img files...\n");
    exit(1);
  }

  assert((BSIZE % sizeof(DInode)) == 0);
  assert((BSIZE % sizeof(Dirent)) == 0);

  fsfd = open(argv[1], O_RDWR|O_CREAT|O_TRUNC, 0666);
  if(fsfd < 0){
    perror(argv[1]);
    exit(1);
  }

  // 1 fs block = 1 disk sector
  nmeta = 2 + nlog + ninodeblocks + nbitmap;
  nblocks = FSSIZE - nmeta;

  sb.size = xuint64(FSSIZE);
  sb.nblocks = xuint64(nblocks);
  sb.ninodes = xuint64(NINODE);
  sb.nlog = xuint64(nlog);
  sb.log_start = xuint64(2);
  sb.inode_start = xuint64(2+nlog);
  sb.bmap_start = xuint64(2+nlog+ninodeblocks);

  printf("nmeta %zu (boot, super, log blocks %zu inode blocks %zu, bitmap blocks %zu) blocks %zu total %llu\n",
         nmeta, nlog, ninodeblocks, nbitmap, nblocks, FSSIZE);

  freeblock = nmeta;     // the first free block that we can allocate

  for(i = 0; i < FSSIZE; i++)
    wsect(i, zeroes);

  memset(buf, 0, sizeof(buf));
  memmove(buf, &sb, sizeof(sb));
  wsect(1, buf);

  rootino = ialloc(FILETYPE_DIR);
  assert(rootino == ROOTINO);

  memset(&de, 0, sizeof(de));
  de.inum = xuint64(rootino);
  de.name[0] = '.';
  iappend(rootino, &de, sizeof(de));

  memset(&de, 0, sizeof(de));
  de.inum = xuint64(rootino);
  de.name[0] = de.name[1] = '.';
  iappend(rootino, &de, sizeof(de));

  for(i = 2; i < argc; i++){
    assert(index(argv[i], '/') == 0);

    if((fd = open(argv[i], 0)) < 0){
      perror(argv[i]);
      exit(1);
    }

    // Skip leading _ in name when writing to file system.
    // The binaries are named _rm, _cat, etc. to keep the
    // build operating system from trying to execute them
    // in place of system binaries like rm and cat.
    if(argv[i][0] == '_')
      ++argv[i];

    inum = ialloc(FILETYPE_FILE);

    memset(&de, 0, sizeof(de));
    de.inum = xuint64(inum);
    strncpy((char *)de.name, argv[i], DIRSIZ);
    iappend(rootino, &de, sizeof(de));

    while((cc = read(fd, buf, sizeof(buf))) > 0)
      iappend(inum, buf, cc);

    close(fd);
  }

  // fix size of root inode dir
  rinode(rootino, &din);
  off = xuint64(din.size);
  off = ((off/BSIZE) + 1) * BSIZE;
  din.size = xuint64(off);
  winode(rootino, &din);

  balloc(freeblock);

  exit(0);
}

void
wsect(uint64_t sec, void *buf)
{
  if(logsec){
    printf("writing sector %llu\n", sec);
  }
  if(lseek(fsfd, sec * BSIZE, 0) != sec * BSIZE){
    perror("lseek");
    exit(1);
  }
  if(write(fsfd, buf, BSIZE) != BSIZE){
    perror("write");
    exit(1);
  }
}

void
winode(uint64_t inum, DInode *ip)
{
  char buf[BSIZE];
  uint64_t bn;
  DInode *dip;

  bn = iblock(&sb, inum);
  rsect(bn, buf);
  dip = ((DInode*)buf) + (inum % IPB);
  memcpy(dip, ip, sizeof(DInode));
  wsect(bn, buf);
}

void
rinode(uint64_t inum, DInode *ip)
{
  char buf[BSIZE];
  uint64_t bn;
  DInode *dip;

  bn = iblock(&sb, inum);
  rsect(bn, buf);
  dip = ((DInode*)buf) + (inum % IPB);
  *ip = *dip;
}

void
rsect(uint64_t sec, void *buf)
{
  if(lseek(fsfd, sec * BSIZE, 0) != sec * BSIZE){
    perror("lseek");
    exit(1);
  }
  if(read(fsfd, buf, BSIZE) != BSIZE){
    perror("read");
    exit(1);
  }
}

uint64_t
ialloc(uint32_t typ)
{
  uint64_t inum = freeinode++;
  DInode din;

  memset(&din, 0, sizeof(din));
  din.typ = xuint32(typ);
  din.nlink = xuint32(1);
  din.size = xuint64(0);
  winode(inum, &din);

  return inum;
}

void
balloc(size_t used)
{
  uint8_t buf[BSIZE];
  int i;

  printf("balloc: first %ld blocks have been allocated\n", used);
  assert(used < BSIZE*8);
  memset(buf, 0, BSIZE);
  for(i = 0; i < used; i++){
    buf[i/8] = buf[i/8] | (0x1 << (i%8));
  }
  printf("balloc: write bitmap block at sector %llu\n", sb.bmap_start);
  wsect(sb.bmap_start, buf);
}

#define min(a, b) ((a) < (b) ? (a) : (b))

void
iappend(uint64_t inum, void *xp, size_t n)
{
  char *p = (char*)xp;
  size_t fbn, off, n1;
  DInode din;
  char buf[BSIZE];
  uint64_t indirect[NINDIRECT];
  uint64_t x, blkno;

  if(logsec && inum == 1){
    for(int i=0; i<32; i++){
      uint8_t *b = xp;
      printf(" %02x", b[i]);
    }
    printf("\n");
  }
  rinode(inum, &din);
  off = xuint64(din.size);
  //printf("append inum %ld at off %ld sz %ld\n", inum, off, n);
  while(n > 0){
    fbn = off / BSIZE;
    assert(fbn < MAXFILE);
    if(fbn < NDIRECT){
      if(din.addrs[fbn] == 0){
        x = freeblock++;
        din.addrs[fbn] = xuint64(x);
      }else{
        x = xuint64(din.addrs[fbn]);
      }
    } else {
      if(din.addrs[NDIRECT] == 0){
        din.addrs[NDIRECT] = xuint64(freeblock++);
      }
      rsect(xuint64(din.addrs[NDIRECT]), (char*)indirect);
      if(indirect[fbn - NDIRECT] == 0){
        indirect[fbn - NDIRECT] = xuint64(freeblock++);
        wsect(xuint64(din.addrs[NDIRECT]), (char*)indirect);
      }
      x = xuint64(indirect[fbn-NDIRECT]);
    }
    n1 = min(n, (fbn + 1) * BSIZE - off);
    rsect(x, buf);
    memcpy(buf + off - (fbn * BSIZE), p, n1);
    wsect(x, buf);
    n -= n1;
    off += n1;
    p += n1;
  }
  din.size = xuint64(off);
  winode(inum, &din);
}
