const int SIGTERM = 15;

static inline int
_kill(int pid, int sig)
{
	extern int kill(int);
	(void)sig;
	return kill(pid);
}
#define kill _kill
