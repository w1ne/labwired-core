#!/bin/bash
set -e

echo "============================================"
echo "🔍 iolinki-master Code Quality & Safety Check"
echo "============================================"

# 1. Compiler Warnings (Strict)
echo -e "\n[1/5] 🛡️  Verifying Compilation Warnings..."
BUILD_DIR="${IOLINKI_MASTER_BUILD_DIR:-build_quality}"
if [ -f "${BUILD_DIR}/CMakeCache.txt" ]; then
    cached_src=$(grep -E '^CMAKE_HOME_DIRECTORY:INTERNAL=' "${BUILD_DIR}/CMakeCache.txt" | cut -d= -f2-)
    if [ -n "${cached_src}" ] && [ "${cached_src}" != "$(pwd)" ]; then
        rm -rf "${BUILD_DIR}"
    fi
fi
mkdir -p "${BUILD_DIR}"
cd "${BUILD_DIR}"
cmake .. -DCMAKE_BUILD_TYPE=Debug \
    -DCMAKE_C_FLAGS="-Wall -Wextra -Werror -Wpedantic -Wconversion -Wshadow"
if make -j"$(nproc)"; then
    echo "   ✅ Strict Compilation Passed"
else
    echo "   ❌ Strict Compilation FAILED"
    exit 1
fi
cd ..

# 2. Static Analysis (Cppcheck)
echo -e "\n[2/5] 🧹 Running Static Analysis (Cppcheck)..."
if command -v cppcheck &> /dev/null; then
    # variableScope suppressed: this stack declares locals at function top by
    # convention; all other warning/style/performance/portability checks stay on.
    if cppcheck --enable=warning,style,performance,portability \
             --error-exitcode=1 \
             --suppress=missingIncludeSystem \
             --suppress=unusedFunction \
             --suppress=variableScope \
             --inline-suppr \
             --quiet \
             -I include \
             src/ examples/; then
        echo "   ✅ Static Analysis Passed"
    else
        echo "   ❌ Static Analysis FAILED"
        exit 1
    fi
else
    echo "   ⚠️ Cppcheck not installed. Skipping static analysis."
    echo "      Install with: sudo apt-get install cppcheck"
fi

# 3. MISRA C:2012 Check (Cppcheck addon)
echo -e "\n[3/5] 📏 Running MISRA C:2012 Check..."
if command -v cppcheck &> /dev/null; then
    MISRA_ADDON=""
    for path in /usr/share/cppcheck/addons/misra.py \
                /usr/lib/cppcheck/addons/misra.py \
                /usr/lib/x86_64-linux-gnu/cppcheck/addons/misra.py; do
        if [ -f "$path" ]; then
            MISRA_ADDON="$path"
            break
        fi
    done

    if [ -n "$MISRA_ADDON" ]; then
        cppcheck --addon=misra \
                 --enable=warning,style,performance,portability \
                 --error-exitcode=1 \
                 --suppress=missingIncludeSystem \
                 --suppress=unusedFunction \
                 --suppress=variableScope \
                 --inline-suppr \
                 --quiet \
                 -I include \
                 src/ examples/
        echo "   ✅ MISRA Check Passed"
    else
        if [ "${IOLINKI_MASTER_MISRA_ENFORCE}" = "1" ]; then
            echo "   ❌ MISRA addon not available in cppcheck. Enforced mode fails."
            exit 1
        else
            echo "   ⚠️ Cppcheck MISRA addon not available. Skipping MISRA checks."
        fi
    fi
else
    echo "   ⚠️ Cppcheck not installed. Skipping MISRA checks."
fi

# 4. Code Formatting Check
echo -e "\n[4/5] 🎨 Checking Code Formatting..."
if command -v clang-format &> /dev/null; then
    if find src include tests examples -type f \( -name "*.c" -o -name "*.h" \) -print0 \
        | xargs -0 clang-format --dry-run --Werror; then
       echo "   ✅ Code Formatting Passed"
    else
       echo "   ❌ Code Formatting FAILED"
       exit 1
    fi
else
    echo "   ⚠️ clang-format not installed. Skipping check."
fi

# 5. Doxygen Warning Check
echo -e "\n[5/5] 📚 Checking Doxygen Warnings..."
if command -v doxygen &> /dev/null; then
    doxygen Doxyfile > /dev/null 2> doxygen.log
    if grep -q "warning:" doxygen.log; then
        echo "   ❌ Doxygen warnings found:"
        grep "warning:" doxygen.log
        exit 1
    else
        echo "   ✅ Doxygen Check Passed"
        rm -f doxygen.log
    fi
else
     echo "   ⚠️ Doxygen not installed. Skipping check."
fi

echo -e "\n============================================"
echo "✅ Code Quality Checks Completed"
echo "============================================"
exit 0
