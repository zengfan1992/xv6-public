#include <sys/stat.h>

#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

int
main(int argc, char *argv[])
{
  int i;

  if(argc < 2){
    dprintf(2, "Usage: mkdir files...\n");
    exit(1);
  }

  for(i = 1; i < argc; i++){
    if(mkdir(argv[i], 0755) < 0){
      dprintf(2, "mkdir: %s failed to create\n", argv[i]);
      exit(1);
    }
  }

  exit(0);
}
