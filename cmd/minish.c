// Shell.

#include <sys/wait.h>

#include <fcntl.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

size_t
gets(char *buf, size_t max)
{
	char c;
	size_t n;
	ssize_t cc;

	for (n = 0; n + 1 < max; ){
		cc = read(0, &c, 1);
		if(cc < 1)
			break;
		buf[n++] = c;
		if(c == '\n' || c == '\r')
			break;
	}
	buf[n] = '\0';

	return n;
}

void
runstressfs(void)
{
	int pid;
	char *args[] = {"stressfs", NULL};

	pid = fork();
	if (pid < 0) {
		printf("fork failed\n");
		exit(1);
	}
	if (pid == 0) {
		execvp("stressfs", args);
		printf("exec failed\n");
		exit(1);
	}
	wait(NULL);
}

void
runls(void)
{
	int pid;
	char *args[] = {"ls", NULL};

	pid = fork();
	if (pid < 0) {
		printf("fork failed\n");
		exit(1);
	}
	if (pid == 0) {
		execvp("ls", args);
		printf("exec failed\n");
		exit(1);
	}
	wait(NULL);
}

void
runpipe(void)
{
	int fds[2];
	int pid;

	if (pipe(fds) < 0) {
		printf("pipe failed\n");
		exit(1);
	}
	pid = fork();
	if (pid < 0) {
		printf("fork failed\n");
		exit(1);
	}
	if (pid == 0) {
		int ch;
		read(fds[0], &ch, 1);
		printf("child read ch='%c'\n", ch);
		close(fds[0]);
		close(fds[1]);
		exit(0);
	}
	if (pid > 0) {
		write(fds[1], "a", 1);
		close(fds[1]);
		close(fds[0]);
		wait(NULL);
	}
}

int
main(int argc, char *argv[])
{
	char buf[128];
	char *nl;
	void *brk;

	printf("argc=%d\n", argc);
	for(int i = 0; i < argc; i++)
		printf("argv[%d] = '%s'\n", i, argv[i]);
	runls();
	runstressfs();
	brk = sbrk(1000);
	printf("brk = %p\n", brk);
	brk = sbrk(-1000);
	printf("brk = %p\n", brk);
	brk = sbrk(4096*16);
	printf("brk = %p\n", brk);
	brk = sbrk(0);
	printf("brk = %p\n", brk);
	runpipe();
	for(;;) {
		printf("$ ");
		if (gets(buf, sizeof(buf)) == 0)
			exit(0);
		nl = strchr(buf, '\n');
		if (nl != NULL)
			*nl = '\0';
		printf("read: '%s'\n", buf);
	}

	return 0;
}
