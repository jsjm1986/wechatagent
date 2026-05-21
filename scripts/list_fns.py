import re, sys
with open('src/routes.rs', encoding='utf-8') as f:
    for i, line in enumerate(f, 1):
        if re.match(r'^(pub )?(async )?fn ', line):
            print(f"{i}: {line.rstrip()}")
