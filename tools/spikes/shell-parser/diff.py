import json, os, random, subprocess, sys, tempfile, collections
from concurrent.futures import ThreadPoolExecutor
random.seed(int(sys.argv[2]) if len(sys.argv)>2 else 1)
N=int(sys.argv[3]) if len(sys.argv)>3 else 3000
mode=sys.argv[1]
root=tempfile.mkdtemp(prefix="sp-"); bindir=root+"/bin"; cwd=root+"/cwd"; os.makedirs(bindir); os.makedirs(cwd)
for n in "abcxyz":
    p=f"{bindir}/{n}"; open(p,"w").write('#!/bin/sh\nprintf "%s\\0" "${0##*/}" "$@"\n'); os.chmod(p,0o755)
TOK=["a","b","c","x","-x","--yy","=","=a","a=b","/","./a",".","%","@",":",",","+","é"," "," "," ","\t","\"","'","\\","$","`","*","?","[","]","{","}","(",")","<",">","|","&",";","!","#","~","\n",
     "$a","${a}","$(a)","`a`","'a b'",'"a b"','"$a"',"'$a'","\\n","a*","{a,b}","~/a","a;b","a|b","a>b","a&","$((1+1))","'a\\'b'",'"a\\"b"',"if","time","!","a~","~","''","\"\"","''~","a'b'~c","x~1","x=~","a:~","a#b","#","x=a~",":~","-- ","--"," -- "]
def gen():
    n=random.randint(1,6); toks=[random.choice(TOK) for _ in range(n)]
    if random.random()<.8: toks.insert(0,random.choice("abcxyz")+" ")
    return "".join(toks)
inputs=list({gen() for _ in range(N)})
res=subprocess.run(["target/release/shell-parser-spike",mode],input="\n".join(json.dumps(s) for s in inputs)+"\n",capture_output=True,text=True).stdout.splitlines()
parsed=[json.loads(r) for r in res]
acc=[(s,a) for s,a in zip(inputs,parsed) if a]
import shutil
SH={"bash":[shutil.which("bash"),"--norc","--noprofile","-c"],"zsh":[shutil.which("zsh"),"-f","-c"],"fish":[shutil.which("fish"),"--no-config","-c"]}
env={"PATH":bindir,"HOME":root,"LC_ALL":"C.UTF-8"}
def run(args):
    sh,s=args
    try: return subprocess.run(SH[sh]+[s],capture_output=True,cwd=cwd,env=env,timeout=5).stdout
    except Exception as e: return ("ERR "+repr(e)).encode()
jobs=[(sh,s) for s,_ in acc for sh in SH]
with ThreadPoolExecutor(8) as ex: outs=list(ex.map(run,jobs))
bad=collections.defaultdict(list)
for i,(s,a) in enumerate(acc):
    exp=("\0".join(a)+"\0").encode() if a[0] in list("abcxyz") else b""
    # first word is looked up by name: expected argv0 is the program name
    for j,sh in enumerate(SH):
        if outs[i*3+j]!=exp: bad[sh].append((s,a,outs[i*3+j]))
print(f"mode={mode} inputs={len(inputs)} accepted={len(acc)} ({100*len(acc)//len(inputs)}%)  mismatching-accepted: "+", ".join(f"{k}={len(v)}" for k,v in bad.items()) + f"  any={len({s for v in bad.values() for s,_,_ in v})}")
shown=set()
for sh,v in bad.items():
    for s,a,o in v[:4]:
        if s in shown: continue
        shown.add(s); print(f"  [{sh}] {s!r} -> ours={a} shell={o!r}")
