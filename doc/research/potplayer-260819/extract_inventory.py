#!/usr/bin/env python3
"""从固定发行包静态提取功能研究索引；不运行 PotPlayer，不推断动态菜单。"""
import argparse
import hashlib
import json
import re
import struct
from collections import Counter, defaultdict
from pathlib import Path


def sha(data):
    return hashlib.sha256(data).hexdigest()


def read_language(path):
    data = path.read_bytes()
    encoding = 'utf-16' if data[:2] in (b'\xff\xfe', b'\xfe\xff') else 'utf-8-sig'
    sections = defaultdict(dict)
    section = None
    for line, text in enumerate(data.decode(encoding).splitlines(), 1):
        if text.startswith('[') and text.endswith(']'):
            section = text[1:-1]
        elif section and '=' in text and not text.startswith(';'):
            key, value = text.split('=', 1)
            if key in sections[section]:
                raise ValueError(f'Duplicate {section}/{key}')
            sections[section][key] = {'text': value, 'line': line}
    return sections, {'file': path.name, 'sha256': sha(data), 'bytes': len(data), 'encoding': encoding}


def split_label(value):
    label, *shortcut = re.split(r'\t|\\t', value, maxsplit=1)
    return label.strip(), shortcut[0].strip() if shortcut else None


def parse_menu(data, menu_id, file_offset):
    version, offset = struct.unpack_from('<HH', data)
    if version != 0:
        raise ValueError('Only standard MENU templates are supported')
    cursor = 4 + offset
    result = []

    def level(parent):
        nonlocal cursor
        index = 0
        while True:
            start = cursor
            flags, = struct.unpack_from('<H', data, cursor)
            cursor += 2
            command = None
            if not flags & 0x10:
                command, = struct.unpack_from('<H', data, cursor)
                cursor += 2
            text_start = cursor
            while struct.unpack_from('<H', data, cursor)[0]:
                cursor += 2
            text = data[text_start:cursor].decode('utf-16-le')
            cursor += 2
            key = f'{parent}_{index}'
            result.append({'key': key, 'command_id': command,
                           'popup': bool(flags & 0x10), 'flags': flags,
                           'separator': command == 0 and not text,
                           'template_text': text,
                           'dll_offset': file_offset + start})
            if flags & 0x10:
                level(key)
            index += 1
            if flags & 0x80:
                break

    level(str(menu_id))
    if cursor != len(data):
        raise ValueError(f'Menu {menu_id}: trailing {len(data) - cursor} bytes')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('release_dir', type=Path, help='7zz 解包后的目录')
    parser.add_argument('--installer', required=True, type=Path)
    parser.add_argument('--output', type=Path, default=Path(__file__).parent)
    args = parser.parse_args()
    resources = json.loads(Path(__file__).with_name('resource-map.json').read_text())
    dll = (args.release_dir / 'PotPlayer64.dll').read_bytes()
    assert sha(dll) == resources['artifact']['sha256'], 'DLL identity mismatch'
    installer_hash = sha(args.installer.read_bytes())
    assert installer_hash == '4b9833adf53b78c2149fe13dd67f2504bf39ee30b45c03f09e5f2c25e43f9cfe'
    en, en_identity = read_language(args.release_dir / 'Language/English.ini')
    zh, zh_identity = read_language(args.release_dir / 'Language/Chinese(Simplified).ini')
    tree = []
    file_version = None
    for resource in resources['resources']:
        location = resource['payload']['location']
        offset, size = location['offset'], location['bytes']
        data = dll[offset:offset + size]
        assert sha(data) == resource['payload']['sha256']
        if resource['type']['id'] == 4:
            tree.extend(parse_menu(data, resource['name']['id'], offset))
        elif resource['type']['id'] == 16:
            signature = data.find(struct.pack('<I', 0xFEEF04BD))
            assert signature >= 0
            _, _, ms, ls = struct.unpack_from('<IIII', data, signature)
            file_version = f'{ms >> 16}.{ms & 65535}.{ls >> 16}.{ls & 65535}'
    by_key = {row['key']: row for row in tree}
    for key in sorted(set(en['MenuString']) | set(zh['MenuString']), key=lambda k: tuple(map(int, k.split('_')))):
        row = by_key.get(key)
        if row is None:
            row = {'key': key, 'command_id': None, 'popup': None,
                   'separator': False, 'dll_offset': None, 'language_only': True}
            tree.append(row)
        for lang, source in [('en', en), ('zh', zh)]:
            record = source['MenuString'].get(key)
            if record:
                row[lang], row[f'shortcut_{lang}'] = split_label(record['text'])
                row[f'line_{lang}'] = record['line']
    by_key = {row['key']: row for row in tree}
    for row in tree:
        ancestors = [row['key'].rsplit('_', n)[0] for n in range(row['key'].count('_'), 0, -1)]
        for lang in ['en', 'zh']:
            row[f'path_{lang}'] = [by_key[k].get(lang, by_key[k].get('en', k)) for k in ancestors if k in by_key] + [row.get(lang, row.get('en', row.get('template_text', row['key']))) ]
    commands = defaultdict(list)
    for row in tree:
        if row['command_id']:
            commands[row['command_id']].append(row['key'])
    dialogs = []
    for section in sorted(set(en) | set(zh)):
        if not re.fullmatch(r'Dialog\(\d+\)', section):
            continue
        for control in sorted(set(en[section]) | set(zh[section]), key=int):
            row = {'section': section, 'control': control}
            for lang, source in [('en', en), ('zh', zh)]:
                value = source[section].get(control)
                if value:
                    row[f'line_{lang}'] = value['line']
                    if len(value['text']) <= 240:
                        row[lang] = value['text']
                    else:
                        row[f'{lang}_omitted'] = {'reason': 'long explanatory text', 'characters': len(value['text']), 'sha256': sha(value['text'].encode())}
            dialogs.append(row)
    strings = []
    for key in sorted(set(en['StringTable']) | set(zh['StringTable']), key=int):
        row = {'key': key}
        for lang, source in [('en', en), ('zh', zh)]:
            value = source['StringTable'].get(key)
            if value:
                row[f'line_{lang}'] = value['line']
                if len(value['text']) <= 240:
                    row[lang] = value['text']
                else:
                    row[f'{lang}_omitted'] = {'reason': 'long explanatory text', 'characters': len(value['text']), 'sha256': sha(value['text'].encode())}
        strings.append(row)
    metadata = {'release': '260819', 'dll_file_version': file_version,
                'installer': {'url': 'https://t1.daumcdn.net/potplayer/PotPlayer/Version/260819/PotPlayerSetup64.exe', 'sha256': installer_hash, 'bytes': args.installer.stat().st_size},
                'languages': [en_identity, zh_identity], 'rea_evidence_id': resources['evidence_id'],
                'counts': {'menu_templates': sum(r['type']['id'] == 4 for r in resources['resources']),
                           'menu_template_nodes': sum(not r.get('language_only') for r in tree),
                           'menu_language_only_nodes': sum(bool(r.get('language_only')) for r in tree),
                           'english_menu_labels': len(en['MenuString']), 'chinese_menu_labels': len(zh['MenuString']),
                           'separators': sum(r['separator'] for r in tree), 'unique_static_command_ids': len(commands),
                           'localized_dialogs': len(set(r['section'] for r in dialogs)), 'dialog_text_records': len(dialogs), 'string_table_records': len(strings)},
                'limitations': ['Static inventory, no Windows execution.', 'Command IDs are scoped to this exact DLL; popup and separator nodes are not commands.', 'Language labels do not prove runtime availability; stale translations may remain.', 'Long dialog descriptions are hashed instead of reproduced.', 'Dynamic menus, combo options and settings tree are not exhaustively recovered.']}
    args.output.mkdir(parents=True, exist_ok=True)
    for name, value in [('metadata.json', metadata), ('menus.json', tree), ('commands.json', dict(sorted(commands.items()))), ('dialogs.json', dialogs), ('strings.json', strings)]:
        (args.output / name).write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n')
    outline = ['# PotPlayer 260819 静态菜单树', '',
               '由固定 DLL 的 MENU 模板与同包中英文资源生成。包含所有非分隔节点，顺序来自模板；不代表实际运行时全部显示。', '',
               'key 是资源路径，CmdID 是此版本实际静态命令 ID；父菜单没有 CmdID。未在语言文件中找到的节点保留占位说明。', '']
    for row in tree:
        if row['separator']:
            continue
        label = row.get('zh') or row.get('en') or '[空标签/动态占位]'
        english = row.get('en', '')
        label = label.replace('&', '').replace('\n', ' ')
        shortcut = row.get('shortcut_en') or row.get('shortcut_zh')
        detail = f"`{row['key']}`"
        if row['command_id']:
            detail += f" · CmdID `{row['command_id']}`"
        if shortcut:
            detail += ' · ' + shortcut.replace('`', '&#96;')
        outline.append('  ' * max(0, row['key'].count('_') - 1) + f'- {label} / {english} — {detail}')
    (args.output / 'menu-tree.md').write_text('\n'.join(outline) + '\n')
    print(json.dumps(metadata, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    main()
