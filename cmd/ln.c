#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>

int
main(int argc, char *argv[])
{
  if(argc != 3){
    dprintf(2, "Usage: ln old new\n");
    exit(1);
  }
  if(link(argv[1], argv[2]) < 0)
    dprintf(2, "link %s %s: failed\n", argv[1], argv[2]);
  exit(0);
}
