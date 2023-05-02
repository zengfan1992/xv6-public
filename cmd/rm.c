#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>

int
main(int argc, char *argv[])
{
  int i;

  if(argc < 2){
    dprintf(2, "Usage: rm files...\n");
    exit(1);
  }

  for(i = 1; i < argc; i++){
    if(unlink(argv[i]) < 0){
      dprintf(2, "rm: %s failed to delete\n", argv[i]);
      exit(1);
    }
  }

  exit(0);
}
