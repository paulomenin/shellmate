import json, subprocess
C="""ls -la
ls -lh /var/log
pwd
uname -a
which python3
git status
git status --short
git log --oneline -n 20
git diff HEAD~1
git branch -a
git checkout -b feature/x-1
git commit -m "fix: parse error"
git push origin main
cargo build --release
cargo test -- --nocapture
cargo run -- --help
docker ps -a
docker run --rm -it ubuntu:24.04 bash
kubectl get pods -n kube-system
systemctl status sshd
journalctl -u nginx --since "1 hour ago"
grep -rn "TODO" src
grep -rn TODO src/
find . -name "*.rs" -type f
find . -name '*.log' -mtime +7
du -sh *
tar -czf out.tar.gz dir/
ssh user@host.example.com
curl -fsSL https://example.com/install.sh
curl -s https://api.github.com/repos/a/b | jq .name
ps aux | grep nginx
cat file.txt | wc -l
echo $HOME
echo "hello world"
rm -rf ./build
cd ~/projects
mv a.txt b.txt
cp -r src/ dst/
chmod +x script.sh
sudo apt update
npm install
npm run build -- --watch
python3 -m http.server 8000
FOO=bar make
make -j8
sed -i 's/a/b/g' file
awk '{print $1}' file
ls > out.txt
ls && pwd
tail -f /var/log/syslog
df -h
free -m
ip addr show
man 7 signal
fd -e rs
rg "fn main" -t rust
""".strip().splitlines()
out=subprocess.run(["target/release/shell-parser-spike","strict"],input="\n".join(json.dumps(c) for c in C)+"\n",capture_output=True,text=True).stdout.splitlines()
ok=[c for c,o in zip(C,out) if o!="null"]; no=[c for c,o in zip(C,out) if o=="null"]
print(f"accepted {len(ok)}/{len(C)}")
print("REJECTED:"); [print("  ",c) for c in no]
