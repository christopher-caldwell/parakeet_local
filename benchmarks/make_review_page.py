"""Build a listen-and-pick page that compares two transcripts of the same audio.

Usage: python3 benchmarks/make_review_page.py RESULT_A.json RESULT_B.json OUTPUT_DIR [--items 60] [--seed 1]

Each RESULT is the output of `parakeet benchmark run` over the same suite. Where the two models
disagree, the page plays a short clip and shows both versions in random order, unlabeled, so the
owner picks what was actually said. No reference text is needed and neither model is favored.
Everything is written under OUTPUT_DIR (keep it under .local/, which is not committed) because the
clips and text are private. Needs ffmpeg. Score the exported choices with score_review.py.
"""
import argparse, difflib, html, json, os, random, re, subprocess

FILLERS = {"um", "uh", "hmm", "mm", "mhm", "erm", "er", "ah", "huh"}

def words(text):
    return re.sub(r"[^a-z0-9' ]", " ", text.lower().replace("-", " ")).split()

def timed_words(run):
    """[(word, start_ms, end_ms)] with times spread evenly across each published segment."""
    out = []
    for piece in run["processing_run"]["result"]["raw"]:
        text, ev = piece.get("text", ""), piece["evidence"]
        ws = words(text)
        if not ws:
            continue
        start, end = ev["local_start_ms"], max(ev["local_end_ms"], ev["local_start_ms"] + 1)
        step = (end - start) / len(ws)
        out += [(w, start + i * step, start + (i + 1) * step) for i, w in enumerate(ws)]
    return out

def items_for(entry, a, b):
    wa, wb = timed_words(a), timed_words(b)
    matcher = difflib.SequenceMatcher(None, [w for w, _, _ in wa], [w for w, _, _ in wb], autojunk=False)
    spans = [op for op in matcher.get_opcodes() if op[0] != "equal"]
    merged = []
    for op in spans:
        if merged and op[1] - merged[-1][2] <= 3:
            merged[-1] = (merged[-1][0], merged[-1][1], op[2], merged[-1][3], op[4])
        else:
            merged.append((op[0], op[1], op[2], op[3], op[4]))
    for _, a1, a2, b1, b2 in merged:
        left, right = [w for w, _, _ in wa[a1:a2]], [w for w, _, _ in wb[b1:b2]]
        if set(left + right) <= FILLERS:
            continue
        lo, hi = max(0, a1 - 6), min(len(wa), a2 + 6)
        around = wa[lo:hi] or wa[max(0, a1 - 1):a1 + 1]
        if not around:
            continue
        t0, t1 = around[0][1] / 1000, around[-1][2] / 1000
        yield {"track": entry, "start": max(0.0, t0 - 0.5), "end": t1 + 0.5,
               "before": " ".join(w for w, _, _ in wa[lo:a1]), "after": " ".join(w for w, _, _ in wa[a2:hi]),
               "a": " ".join(left), "b": " ".join(right)}

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("a"); ap.add_argument("b"); ap.add_argument("out")
    ap.add_argument("--items", type=int, default=60); ap.add_argument("--seed", type=int, default=1)
    args = ap.parse_args()
    ra = {r["entry"] if isinstance(r["entry"], str) else r["entry"]["id"]: r for r in json.load(open(args.a))["data"]["results"]}
    rb = {r["entry"] if isinstance(r["entry"], str) else r["entry"]["id"]: r for r in json.load(open(args.b))["data"]["results"]}
    suite = json.load(open(args.a))["data"]["suite"]["entries"]
    audio = {e["id"]: e["audio"] for e in suite}
    pool = []
    for key in ra:
        if key in rb and "processing_run" in ra[key] and "processing_run" in rb[key]:
            pool += list(items_for(key, ra[key], rb[key]))
    rng = random.Random(args.seed)
    rng.shuffle(pool)
    chosen = pool[: args.items]
    os.makedirs(f"{args.out}/clips", exist_ok=True)
    cards = []
    for n, item in enumerate(chosen):
        clip = f"clips/{n:03d}.wav"
        subprocess.run(["ffmpeg", "-y", "-nostdin", "-v", "error", "-ss", f"{item['start']:.2f}", "-t", f"{item['end'] - item['start']:.2f}",
                        "-i", audio[item["track"]], "-map", "0:a:0", "-ac", "1", "-ar", "16000", f"{args.out}/{clip}"], check=True)
        flip = rng.random() < 0.5
        one, two = (item["b"], item["a"]) if flip else (item["a"], item["b"])
        cards.append({"id": n, "clip": clip, "before": item["before"], "after": item["after"], "one": one, "two": two, "flip": flip})
    page = PAGE.replace("__CARDS__", json.dumps(cards)).replace("__TOTAL_POOL__", str(len(pool)))
    open(f"{args.out}/review.html", "w").write(page)
    json.dump({"flips": {c["id"]: c["flip"] for c in cards}, "pool": len(pool)}, open(f"{args.out}/key.json", "w"))
    print(f"{len(chosen)} items from {len(pool)} disagreements -> {args.out}/review.html")

PAGE = r"""<!doctype html><meta charset=utf-8><meta name=viewport content="width=device-width,initial-scale=1">
<title>Which version is right?</title>
<style>body{font:16px/1.5 system-ui;max-width:760px;margin:24px auto;padding:0 16px;color:#222;background:#fff}
.card{border:1px solid #ccc;border-radius:10px;padding:14px;margin:14px 0}.done{background:#f3faf3}
.ctx{color:#777}.v{margin:6px 0}.pick{display:flex;gap:8px;flex-wrap:wrap;margin-top:8px}
button{font:inherit;padding:6px 12px;border-radius:8px;border:1px solid #888;background:#f6f6f6;cursor:pointer}
button.sel{background:#1f6feb;color:#fff;border-color:#1f6feb}input[type=text]{width:100%;padding:6px;margin-top:6px;font:inherit}
@media(prefers-color-scheme:dark){body{background:#161616;color:#eee}.card{border-color:#444}.done{background:#1c261c}button{background:#2a2a2a;color:#eee}}</style>
<h1>Which version is right?</h1>
<p>Listen to each short clip, then pick the version that matches what was said. Pick <b>Both fine</b> if they mean the same, or <b>Neither</b> and type the correct words. Skip any you can't make out. The versions are in random order. About <span id=n></span> clips, chosen from __TOTAL_POOL__ places where the two transcripts differ.</p>
<div id=list></div><p><button id=save>Save my answers</button> <span id=count></span></p>
<script>
const cards=__CARDS__;let ans={};try{ans=JSON.parse(localStorage.getItem('review')||'{}')}catch(e){}
document.getElementById('n').textContent=cards.length;
const list=document.getElementById('list');
function store(){try{localStorage.setItem('review',JSON.stringify(ans))}catch(e){};count()}
function count(){document.getElementById('count').textContent=Object.keys(ans).length+' of '+cards.length+' answered'}
cards.forEach(c=>{const d=document.createElement('div');d.className='card';
d.innerHTML=`<audio controls preload=none src="${c.clip}"></audio><div class=ctx>…${c.before}</div>
<div class=v><b>1:</b> ${c.one||'<i>(nothing)</i>'}</div><div class=v><b>2:</b> ${c.two||'<i>(nothing)</i>'}</div><div class=ctx>${c.after}…</div><div class=pick></div>`;
const pick=d.querySelector('.pick');
[['1','Version 1'],['2','Version 2'],['both','Both fine'],['neither','Neither'],['skip','Skip']].forEach(([k,t])=>{const b=document.createElement('button');b.textContent=t;
b.onclick=()=>{ans[c.id]={choice:k,text:ans[c.id]&&ans[c.id].text||''};render();store()};b.dataset.k=k;pick.appendChild(b)});
const inp=document.createElement('input');inp.type='text';inp.placeholder='If neither: type what was said';inp.oninput=()=>{ans[c.id]={choice:'neither',text:inp.value};render();store()};d.appendChild(inp);
c.el=d;c.inp=inp;list.appendChild(d)});
function render(){cards.forEach(c=>{const a=ans[c.id];c.el.classList.toggle('done',!!a);c.el.querySelectorAll('button').forEach(b=>b.classList.toggle('sel',!!a&&a.choice===b.dataset.k));if(a&&a.text)c.inp.value=a.text})}
render();count();
document.getElementById('save').onclick=()=>{const blob=new Blob([JSON.stringify(ans,null,1)],{type:'application/json'});const a=document.createElement('a');a.href=URL.createObjectURL(blob);a.download='review-answers.json';a.click()};
</script>"""

if __name__ == "__main__":
    main()
