"""Build single-speaker meeting tracks from AMI headset audio (edinburghcstr/ami, CC BY 4.0).

Usage: python3 benchmarks/prepare_ami.py PARQUET OUTPUT_DIR   (needs numpy, soundfile, pyarrow)
Each track is one speaker placed at original times (10 min from 5:00), so most of it is silence
or other people. Fillers and truncated words are removed from the reference. Writes suite.json.
"""
import io,json,re,sys,collections
import os
import numpy as np, soundfile as sf, pyarrow.parquet as pq
PARQUET,OUT=sys.argv[1:3]; WINDOW=(300.0,900.0)  # ten minutes of each speaker's headset track starting at 5:00
t=pq.read_table(PARQUET).to_pylist()
os.makedirs(OUT,exist_ok=True)
FILL={"um","uh","hmm","mm","mhm","uh-huh","mm-hmm","ah","er","erm","huh","hm","oh","uhhuh"}
def clean(txt):
    out=[]
    for w in txt.lower().split():
        if w in FILL or w.endswith("-") or w.startswith("<") or w.startswith("["): continue
        out.append(w)
    return " ".join(out)
by=collections.defaultdict(list)
for r in t:
    if WINDOW[0]<=r["begin_time"]<WINDOW[1]: by[(r["meeting_id"],r["speaker_id"])].append(r)
top=sorted(by.items(),key=lambda kv:-sum(len(r["text"].split()) for r in kv[1]))[:5]
entries=[]
for (m,s),rows in top:
    rows.sort(key=lambda r:r["begin_time"])
    sr=16000;buf=np.zeros(int((WINDOW[1]-WINDOW[0]+30)*sr),dtype=np.float32)
    ref=[]
    for r in rows:
        x,rate=sf.read(io.BytesIO(r["audio"]["bytes"]),dtype="float32")
        if x.ndim>1:x=x.mean(1)
        assert rate==sr,rate
        a=int((r["begin_time"]-WINDOW[0])*sr);buf[a:a+len(x)]+=x[:len(buf)-a]
        c=clean(r["text"]); 
        if c: ref.append(c)
    path=f"{OUT}/{m}-{s}.wav";sf.write(path,np.clip(buf,-1,1),sr,subtype="PCM_16")
    words=sum(len(x.split()) for x in ref)
    entries.append({"id":f"ami-{m}-{s}","audio":path,"reference":None,"reference_text":" ".join(ref)})
    print(m,s,len(rows),"utts",words,"words")
base={"split":"development","labels":"verified_public","models":["whisper-small.en"],"collar_ms":0,"score_overlap":True,"word_normalization":"english_case_punctuation"}
base.update({"id":"ami-ihm-single-speaker-tracks","dataset":"AMI corpus headset mics (edinburghcstr/ami ihm test, CC BY 4.0); one speaker's track, 10 min from 5:00; fillers and truncated words removed from reference","entries":entries})
json.dump(base,open(OUT+"/suite.json","w"),indent=1)
