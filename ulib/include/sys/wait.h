static inline int
_wait(int *p)
{
	int wait(void);

	(void)p;
	return wait();
}
#define wait _wait
