/* Test-emulator only: credit the idle TCG guest's entropy pool (RNDADDENTROPY). */
#include <fcntl.h>
#include <linux/random.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/ioctl.h>
#include <unistd.h>
int main(void) {
  struct { int entropy_count; int buf_size; unsigned char buf[512]; } e;
  int src = open("/dev/urandom", O_RDONLY), dst = open("/dev/random", O_WRONLY);
  if (src < 0 || dst < 0 || read(src, e.buf, sizeof e.buf) != sizeof e.buf) { perror("open/read"); return 1; }
  e.buf_size = sizeof e.buf; e.entropy_count = 8 * sizeof e.buf;
  if (ioctl(dst, RNDADDENTROPY, &e)) { perror("RNDADDENTROPY"); return 1; }
  int bits = 0; ioctl(dst, RNDGETENTCNT, &bits); printf("entropy_avail=%d\n", bits); return 0;
}
