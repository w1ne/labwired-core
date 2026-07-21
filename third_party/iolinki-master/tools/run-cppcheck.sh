#!/bin/bash
# Wrapper for cppcheck to be used with pre-commit and check_quality.sh.
# It ensures correct include paths and suppression settings.
#
# Pre-commit passes the list of files as arguments.

set -e

# variableScope is suppressed on purpose: this stack declares locals at function
# top (a deliberate, consistent convention), which cppcheck's style check would
# otherwise flag. All other warning/style/performance/portability checks stay on.
cppcheck --enable=warning,style,performance,portability \
         --error-exitcode=1 \
         --suppress=missingIncludeSystem \
         --suppress=unusedFunction \
         --suppress=variableScope \
         --inline-suppr \
         --quiet \
         -I include \
         "$@"
