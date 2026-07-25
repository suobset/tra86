#include <stdio.h>

int helper(int x) {
    return x + 42;
}

int main(void) {
    int total = 0;
    for (int i = 0; i < 3; i++) {
        total += helper(i);
    }
    printf("total=%d\n", total);
    return 0;
}
