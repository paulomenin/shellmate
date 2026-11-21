"""Self-contained, escaped HTML review report and Markdown summary."""
import html
import json
import statistics
import math
from pathlib import Path


def summarize(runs):
    passed = sum(r.get("passed") is True for r in runs)
    priced = [r["cost_usd"] for r in runs if r.get("cost_usd") is not None]
    times = sorted(r["timings"]["llm_ms"] for r in runs if r.get("timings", {}).get("llm_ms") is not None)
    unsafe = sum(any(k["name"] == "safe" and not k["ok"] for k in r.get("checks", [])) for r in runs)
    complete_cost = len(priced) == len(runs) and bool(runs)
    position = (len(times) - 1) * .95 if times else 0
    p95 = times[math.floor(position)] + (times[math.ceil(position)] - times[math.floor(position)]) * (position % 1) if times else None
    return {"runs": len(runs), "passed": passed, "pass_rate": passed / len(runs) if runs else None,
            "unsafe_flags": unsafe, "errors": sum(bool(r.get("error")) for r in runs),
            "behavior_unreviewed": sum(r.get("behavior_status") == "not_approved" for r in runs),
            "cost_usd": sum(priced) if complete_cost else None,
            "known_cost_usd": sum(priced), "priced_runs": len(priced),
            "cost_per_pass_usd": sum(priced) / passed if complete_cost and passed else None,
            "cost_per_1000_usd": sum(priced) / len(runs) * 1000 if complete_cost else None,
            "llm_p50_ms": statistics.median(times) if times else None,
            "llm_p95_ms": p95,
            "prose_words_p50": statistics.median([r["style"]["prose_words"] for r in runs if "style" in r]) if any("style" in r for r in runs) else None,
            "decision_coverage": sum(r.get("decision_metrics", {}).get("covered", False) for r in runs) / len(runs) if runs and any("decision_metrics" in r for r in runs) else None,
            "missed_destructive": sum(r.get("decision_metrics", {}).get("missed_destructive", False) for r in runs),
            "false_mismatch_alarms": sum(r.get("decision_metrics", {}).get("false_mismatch_alarm", False) for r in runs),
            "missed_mismatches": sum(r.get("decision_metrics", {}).get("missed_mismatch", False) for r in runs),
            "brier": statistics.mean([r["decision_metrics"]["brier"] for r in runs if "brier" in r.get("decision_metrics", {})]) if any("brier" in r.get("decision_metrics", {}) for r in runs) else None}


def groups(result):
    out = {}
    for r in result["runs"]:
        if r.get("warmup"):
            continue
        out.setdefault(r["profile"], []).append(r)
    return out


def markdown(result):
    lines = ["# Model evaluation", "", f"Suite: `{result['suite_hash']}`", "",
             "Scores cover explicit rules only. Unapproved behavioral checks remain incomplete. Replay timings and costs are not model measurements.", "",
             "| Profile | Passed | Errors | Unsafe flags | Cost USD | Cost/pass USD | USD/1,000 | Median model ms | p95 ms | Median prose words |",
             "| :--- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"]
    for name, runs in groups(result).items():
        s = summarize(runs)
        fmt = lambda v: "unknown" if v is None else f"{v:.6g}"
        escaped = name.replace("|", "\\|").replace("\n", " ")
        lines.append(f"| {escaped} | {s['passed']}/{s['runs']} | {s['errors']} | {s['unsafe_flags']} | {fmt(s['cost_usd'])} | {fmt(s['cost_per_pass_usd'])} | {fmt(s['cost_per_1000_usd'])} | {fmt(s['llm_p50_ms'])} | {fmt(s['llm_p95_ms'])} | {fmt(s['prose_words_p50'])} |")
    lines += ["", "Category, decider, usage, behavior and per-answer details are in review.html and results.json."]
    for name, runs in groups(result).items():
        lines += ["", f"## {name}", "", "| Category | Passed |", "| :--- | ---: |"]
        for category in sorted({r["category"] for r in runs}):
            selected = [r for r in runs if r["category"] == category]
            lines.append(f"| {category} | {sum(r['passed'] is True for r in selected)}/{len(selected)} |")
        decisions = summarize(runs)
        if decisions["decision_coverage"] is not None:
            lines += ["", "Decider metrics: " + json.dumps({k: decisions[k] for k in ("decision_coverage", "missed_destructive", "missed_mismatches", "false_mismatch_alarms", "brier")})]
    warmups = [r for r in result["runs"] if r.get("warmup")]
    if warmups:
        lines += ["", f"Unscored first requests: {len(warmups)}. Their usage, cost and first-request times are preserved in results.json."]
    return "\n".join(lines) + "\n"


def render(result, blind=False):
    e = lambda s: html.escape(str(s), quote=True)
    aliases = {name: f"Model {i + 1}" for i, name in enumerate(groups(result))}
    cards, review = [], {"schema": 1, "suite_hash": result["suite_hash"], "evaluation_id": result.get("evaluation_id"), "ratings": {}}
    for r in result["runs"]:
        title = aliases.get(r["profile"], "Warm-up") if blind else r["profile"]
        # Omit identifying metadata in blind mode; keep answer text unmodified for fair review.
        detail = {k: r[k] for k in ("expect", "behavior_expect", "decision_expected", "checks", "style", "raw_answer", "decision", "decision_metrics", "behavior", "behavior_status", "error") if k in r}
        detail["fixture_files"] = {k: v for k, v in result.get("fixtures", {}).items() if k.startswith(r.get("fixture", "") + "/")}
        if not blind:
            detail.update({k: r[k] for k in ("timings", "cost_usd", "calls") if k in r})
        cards.append(f'''<article data-id="{e(r['id'])}"><h2>{e(title)} · {e(r['case_id'])} · repeat {r['repeat'] + 1}</h2>
<p>{e(r['action'])} / {e(r['shell'])} · automated result: {e(r.get('passed'))}</p>
<h3>Request</h3><pre>{e(r['input'])}</pre><h3>Answer</h3><pre>{e(r.get('output', ''))}</pre>
<h3>Suggested command</h3><pre>{e(r.get('command', ''))}</pre>
<details><summary>Checks and measurements</summary><pre>{e(json.dumps(detail, indent=2))}</pre></details>
<label>Human rating <select><option value="">Unreviewed</option><option>correct</option><option>partially_correct</option><option>incorrect</option><option>unsafe</option><option>unnecessary_refusal</option></select></label>
<label>Notes <textarea rows="3"></textarea></label></article>''')
        review["ratings"][r["id"]] = {"rating": "", "notes": ""}
    # JSON lives in inert text; escape '<' so model text can never terminate a script element.
    seed = json.dumps(review).replace("<", "\\u003c")
    return '''<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'none'; img-src 'none'; base-uri 'none'; form-action 'none'">
<title>Model evaluation review</title><style>body{font:16px system-ui;max-width:1000px;margin:2rem auto;padding:0 1rem;background:#fafafa;color:#222}article{border:1px solid #ccc;border-radius:8px;padding:1rem;margin:1rem 0;background:white}pre{white-space:pre-wrap;overflow-wrap:anywhere;background:#f0f0f0;padding:.8rem}label{display:block;margin:.8rem 0}textarea{display:block;width:95%}button,input{margin:.5rem}</style>
<h1>Model evaluation review</h1><p>Review all answers. Ratings stay in this page until you export them. Import a prior export to resume. Blinding hides metadata, not model self-identification in answers.</p>
<button id="export">Export review JSON</button><label>Resume review <input id="import" type="file" accept="application/json"></label><p id="status"></p>
''' + "".join(cards) + f'''<script type="application/json" id="seed">{seed}</script>''' + '''<script>
const review=JSON.parse(document.getElementById('seed').textContent);
const cards=[...document.querySelectorAll('article')];
document.getElementById('export').onclick=()=>{
 for(const card of cards)review.ratings[card.dataset.id]={rating:card.querySelector('select').value,notes:card.querySelector('textarea').value};
 const url=URL.createObjectURL(new Blob([JSON.stringify(review,null,2)],{type:'application/json'}));
 const a=document.createElement('a');a.href=url;a.download='review.json';a.click();setTimeout(()=>URL.revokeObjectURL(url),1000);
};
document.getElementById('import').onchange=async event=>{
 try{const data=JSON.parse(await event.target.files[0].text());if(data.suite_hash!==review.suite_hash||data.evaluation_id!==review.evaluation_id)throw Error('Evaluation differs');
 for(const card of cards){const rating=data.ratings[card.dataset.id];if(rating){card.querySelector('select').value=rating.rating;card.querySelector('textarea').value=rating.notes;}}
 document.getElementById('status').textContent='Review imported';}catch(error){document.getElementById('status').textContent='Cannot import this review';}
};</script></html>'''


def write(result, directory, blind=False):
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    (directory / "results.json").write_text(json.dumps(result, indent=2) + "\n")
    (directory / "summary.md").write_text(markdown(result))
    summary = {}
    for name, runs in groups(result).items():
        summary[name] = {**summarize(runs), "by_category": {category: summarize([r for r in runs if r["category"] == category]) for category in sorted({r["category"] for r in runs})}}
    (directory / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    (directory / "review.html").write_text(render(result, blind))
