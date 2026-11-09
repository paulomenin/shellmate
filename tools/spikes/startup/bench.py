import subprocess, time, statistics, sys
def bench(cmd, n=300):
    ts=[]
    for _ in range(n):
        t=time.perf_counter(); subprocess.run(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL); ts.append((time.perf_counter()-t)*1000)
    ts.sort(); return ts[len(ts)//2], ts[int(len(ts)*.95)], ts[0]
B='target/release/'
for name,cmd in [('true (floor)',['/usr/bin/true']),('a_min',[B+'a_min']),('b_sync parse',[B+'b_sync','config.toml']),('c_tokio parse',[B+'c_tokio','config.toml'])]:
    p50,p95,mn=bench(cmd); print(f'{name:16} p50={p50:5.2f} ms  p95={p95:5.2f}  min={mn:5.2f}')
