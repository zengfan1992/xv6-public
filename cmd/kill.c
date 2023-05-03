#include <signal.h>
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>

int
main(int argc, char *argv[])
{
  int i;

  if(argc < 2){
    dprintf(2, "usage: kill pid...\n");
    exit(1);
  }
  for(i=1; i<argc; i++)
    kill(atoi(argv[i]), SIGTERM);
  exit(0);
}
