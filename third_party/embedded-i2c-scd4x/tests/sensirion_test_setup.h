#ifndef SENSIRION_TEST_SETUP_H
#define SENSIRION_TEST_SETUP_H

#include <stdio.h>

#include "CppUTest/TestHarness.h"

// provide macros which will only be available in a future cpputest release
// See https://github.com/cpputest/cpputest/pull/1423
#ifndef CHECK_EQUAL_ZERO
#define CHECK_EQUAL_ZERO(actual) CHECK_EQUAL(0, (actual))
#endif
#ifndef CHECK_EQUAL_ZERO_TEXT
#define CHECK_EQUAL_ZERO_TEXT(actual, text) \
    CHECK_EQUAL_TEXT(0, (actual), (text))
#endif

int main(int argc, char** argv);

#endif /* SENSIRION_TEST_SETUP_H */
