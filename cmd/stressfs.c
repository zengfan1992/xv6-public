// Demonstrate that moving the "acquire" in iderw after the loop that
// appends to the idequeue results in a race.

// For this to work, you should also add a spin within iderw's
// idequeue traversal loop.  Adding the following demonstrated a panic
// after about 5 runs of stressfs in QEMU on a 2.1GHz CPU:
//    for (i = 0; i < 40000; i++)
//      asm volatile("");

#include <sys/wait.h>

#include <fcntl.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

int
main(int argc, char *argv[])
{
  int fd, i;
  char path[] = "stressfs0";
  char r[] = "read 0\n";
  char w[] = "write 0\n";
  char data[512];

  printf("stressfs starting\n");
  memset(data, 'a', sizeof(data));

  for(i = 0; i < 4; i++)
    if(fork() > 0)
      break;

  w[6] += i;
  r[5] += i;
  path[8] += i;

  write(1, w, strlen(w));
  fd = open(path, O_CREAT | O_RDWR);
  for(i = 0; i < 20; i++){
    dprintf(fd, "%d\n", i);
    write(fd, data, sizeof(data));
  }
  close(fd);

  write(1, r, strlen(r));
  fd = open(path, O_RDONLY);
  for (i = 0; i < 20; i++)
    read(fd, data, sizeof(data));
  close(fd);

  wait(NULL);

  exit(0);
}
