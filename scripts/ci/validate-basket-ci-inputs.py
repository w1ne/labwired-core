#!/usr/bin/env python3
"""Fail closed on opt-in private sibling checkout prerequisites; never print tokens."""
import os
import re
import sys
commit = os.environ.get('LABWIRED_BASKET_APP_COMMIT', '')
if not re.fullmatch(r'[0-9a-f]{40}', commit):
    sys.exit('basket_app_commit must be an explicit full lowercase 40-hex app commit; moving refs are refused')
if not os.environ.get('LABWIRED_BASKET_APP_READ_TOKEN', ''):
    sys.exit('Opt-in basket acceptance needs LABWIRED_BASKET_APP_READ_TOKEN with read access to private w1ne/labwired; core GITHUB_TOKEN is insufficient')
print('Validated pinned private app commit: ' + commit)
