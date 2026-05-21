import sys
lines = open('src/routes.rs', encoding='utf-8').readlines()
sys.stdout.write(f"total={len(lines)}\n")
