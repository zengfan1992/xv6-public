#include <sys/stat.h>
#include <sys/wait.h>

#include <fcntl.h>
#include <signal.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

void
mem(void)
{
  void *m1, *m2;
  int pid, ppid;

  printf("mem test\n");
  ppid = getpid();
  if((pid = fork()) == 0){
    m1 = 0;
    while((m2 = malloc(1000001)) != 0){
      *(char**)m2 = m1;
      m1 = m2;
    }
    while(m1){
      m2 = *(char**)m1;
      free(m1);
      m1 = m2;
    }
    m1 = malloc(1024*1024*2+1);
    if(m1 == 0){
      printf("couldn't allocate mem?!!\n");
      kill(ppid, SIGTERM);
      exit(1);
    }
    free(m1);
    printf("mem ok\n");
    exit(1);
  } else {
    wait(NULL);
  }
}

int
main(int argc, char *argv[])
{
  printf("malloctest starting\n");

  mem();

  exit(0);
}
