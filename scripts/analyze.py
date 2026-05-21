"""Analyze routes.rs - find top-level items and their line ranges."""
import re

with open('src/routes.rs', encoding='utf-8') as f:
    lines = f.readlines()

# Find top-level items (lines starting with no indentation, not "}")
# We track depth via braces
items = []  # (start_line, end_line, kind, name, signature)
i = 0
depth = 0
in_doc = False
doc_start = None
n = len(lines)
while i < n:
    line = lines[i]
    # Strip but track meaningful content
    stripped = line.lstrip()
    is_top_level = (depth == 0 and stripped and not line.startswith(' ') and not line.startswith('\t'))
    
    if depth == 0:
        # Start tracking doc comments
        if stripped.startswith('//') or stripped.startswith('#['):
            if doc_start is None:
                doc_start = i
        elif is_top_level and re.match(r'(pub )?(async )?fn |pub fn |fn |struct |pub struct |enum |pub enum |use |#\[', line):
            # Item start
            start = doc_start if doc_start is not None else i
            # Determine kind/name
            m = re.match(r'(pub )?(async )?fn (\w+)', line)
            if m:
                kind = 'fn'
                name = m.group(3)
            else:
                m = re.match(r'(pub )?struct (\w+)', line)
                if m:
                    kind = 'struct'
                    name = m.group(2)
                else:
                    m = re.match(r'(pub )?enum (\w+)', line)
                    if m:
                        kind = 'enum'
                        name = m.group(2)
                    else:
                        # don't track this
                        kind = None
                        name = None
            
            # Now find end by matching braces
            # First locate first '{' in current or future line
            search_i = i
            opening = None
            while search_i < n:
                if '{' in lines[search_i]:
                    # find first {
                    opening = search_i
                    break
                if ';' in lines[search_i] and search_i > i:
                    # struct decl with no body? unlikely
                    break
                search_i += 1
            if opening is None:
                # like 'use ...;' single line
                if kind:
                    items.append((start+1, i+1, kind, name))
                doc_start = None
                i += 1
                continue
            # Count braces from opening
            bd = 0
            end = opening
            for j in range(opening, n):
                for ch in lines[j]:
                    if ch == '{':
                        bd += 1
                    elif ch == '}':
                        bd -= 1
                        if bd == 0:
                            end = j
                            break
                if bd == 0 and end == j:
                    break
            if kind:
                items.append((start+1, end+1, kind, name))
            doc_start = None
            i = end + 1
            continue
        else:
            doc_start = None
    i += 1

for it in items:
    print(it)
