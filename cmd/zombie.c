// Create a zombie process that
// must be reparented at exit.

#include <stddef.h>
#include <stdlib.h>
#include <unistd.h>

int
main(void)
{
  if(fork() > 0)
    sleep(500);  // Let child exit before parent.
  exit(0);
}
