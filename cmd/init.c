// init: The initial user-level program

#include <sys/stat.h>
#include <sys/wait.h>

#include <fcntl.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>

char *argv[] = { "sh", "hi", "there", "test", NULL };

int
main(void)
{
  int pid, wpid;

  if(open("console", O_RDWR) < 0){
    mknod("console", 0, 0);
    open("console", O_RDWR);
  }
  dup(0);  // stdout
  dup(0);  // stderr

  for(;;){
    printf("init: starting sh\n");
    pid = fork();
    if(pid < 0){
      printf("init: fork failed\n");
      exit(1);
    }
    if(pid == 0){
      execvp("sh", argv);
      printf("init: exec sh failed\n");
      exit(1);
    }
    while((wpid=wait(NULL)) >= 0 && wpid != pid)
      printf("zombie!\n");
  }
}
