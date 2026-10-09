"""Read the deliberately small mapping/scalar YAML subset used by the wire contract.

No aliases, tags, block strings, implicit expressions or external dependencies.
Unsupported syntax is an error rather than an ignored expectation.
"""
import json
import re


def _parts(text):
    parts=[];start=0;depth=0;quote=None;escaped=False
    for index,char in enumerate(text):
        if quote:
            if escaped:escaped=False
            elif char=='\\' and quote=='"':escaped=True
            elif char==quote:quote=None
        elif char in "'\"":quote=char
        elif char in '[{':depth+=1
        elif char in ']}':
            depth-=1
            if depth<0:raise ValueError('unbalanced contract flow value')
        elif char==',' and depth==0:parts.append(text[start:index].strip());start=index+1
    if quote or depth:raise ValueError('unclosed contract flow value')
    parts.append(text[start:].strip())
    if any(not part for part in parts):raise ValueError('empty contract flow item')
    return parts


def _key(text):
    if not re.fullmatch(r'[A-Za-z_][A-Za-z0-9_-]*',text):raise ValueError('unsupported contract key')
    return text


def _scalar(text,depth=0):
    if depth>16:raise ValueError('contract nesting limit')
    if text.startswith('['):
        if not text.endswith(']'):raise ValueError('unclosed contract list')
        return [] if text=='[]' else [_scalar(part,depth+1) for part in _parts(text[1:-1])]
    if text.startswith('{'):
        if not text.endswith('}'):raise ValueError('unclosed contract map')
        result={}
        if text=='{}':return result
        for part in _parts(text[1:-1]):
            key,colon,value=part.partition(':');key=_key(key.strip())
            if not colon or not value.strip() or key in result:raise ValueError('invalid or duplicate contract flow key')
            result[key]=_scalar(value.strip(),depth+1)
        return result
    if text.startswith('"'):
        value=json.loads(text)
        if not isinstance(value,str):raise ValueError('quoted contract value must be string')
        return value
    if text.startswith("'"):
        if not re.fullmatch(r"'(?:[^']|'')*'",text):raise ValueError('invalid quoted contract scalar')
        return text[1:-1].replace("''", "'")
    if re.fullmatch(r'0|[1-9][0-9]*',text):return int(text)
    if text in ('true','false'):return text=='true'
    if text=='null':return None
    if not text or any(char in text for char in "[]{}'\"#&*!|>\t") or any(ord(char)<32 for char in text):
        raise ValueError('unsupported contract scalar syntax')
    return text


def read_contract(path):
    raw=path.read_text(encoding='utf-8')
    if len(raw.encode('utf-8'))>65536:raise ValueError('contract exceeds 64 KiB')
    result={};stack=[(-2,result)]
    for line in raw.splitlines():
        if not line.strip() or line.lstrip().startswith('#'):continue
        if '\t' in line:raise ValueError('contract tabs are unsupported')
        indent=len(line)-len(line.lstrip(' '))
        if indent%2 or indent>32:raise ValueError('invalid contract indentation')
        while stack[-1][0]>=indent:stack.pop()
        if indent!=stack[-1][0]+2:raise ValueError('unexpected contract indentation')
        key,colon,value=line.strip().partition(':');key=_key(key)
        if not colon or key in stack[-1][1]:raise ValueError('invalid or duplicate contract key')
        value=value.strip()
        if value:stack[-1][1][key]=_scalar(value)
        else:
            child={};stack[-1][1][key]=child;stack.append((indent,child))
    if not result:raise ValueError('empty contract')
    return result
