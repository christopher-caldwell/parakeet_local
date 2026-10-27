#!/usr/bin/env python3
"""Local embedding/enrollment smoke test using published qpylu speaker labels.
Labels refer to speakers within one public recording, not real-world names.
No embeddings or audio are printed or published. This is not cross-meeting accuracy evidence.
"""
import json,pathlib,subprocess,os
root=pathlib.Path('.local/app');db=pathlib.Path(os.environ.get('RECOGNITION_DB','.local/recognition.db'));out=pathlib.Path(os.environ.get('RECOGNITION_OUT','.local/recognition'));out.mkdir(exist_ok=True)
base=['target/debug/parakeet','--data-root',str(root),'--database',str(db)]
def run(*args):
 p=subprocess.run([*base,*args],capture_output=True);v=json.loads(p.stdout)
 if p.returncode:raise RuntimeError(v.get('error'))
 return v['data']
if not db.exists():run('db','create')
run('db','migrate')
r=run('meetings','import','.local/public/voxconverse/qpylu.wav')['recordings'][0];detail=run('meetings','show',r);track=detail['tracks'][0]['id']
profiles=run('speakers','list');profile=next((p for p in profiles if p['name']=='Public fixture qpylu spk00'),None) or run('speakers','create','Public fixture qpylu spk00');pid=profile['id']
examples=run('speakers','examples',pid)
for a,b in [(32840,35840),(35840,38840)]:
 if not any(e['start_ms']==a and e['end_ms']==b for e in examples):run('speakers','enroll',pid,r,'--track',track,'--start-ms',str(a),'--end-ms',str(b),'--confirmed')
truth=json.loads(pathlib.Path('.local/public/voxconverse/qpylu.reference.json').read_text());negative=max((s for s in truth if s['speaker']!='spk00'),key=lambda s:s['end_ms']-s['start_ms']);a=negative['start_ms']+200;b=min(negative['end_ms']-200,a+4000)
policy=['--minimum-score','0.9','--minimum-margin','0.05','--minimum-examples','2']
positive=run('speakers','suggest',r,'--track',track,'--start-ms','39040','--end-ms','43040',*policy)
negative_decision=run('speakers','suggest',r,'--track',track,'--start-ms',str(a),'--end-ms',str(b),*policy)
# The first implementation returns the decision directly; future receipts may wrap it.
pdecision=positive.get('decision',positive);ndecision=negative_decision.get('decision',negative_decision)
report={'protocol':'same-recording public-label smoke, two disjoint enrollment samples and disjoint query; no threshold tuning','profile_alias':'qpylu/spk00','verified_examples':len(run('speakers','examples',pid)),'policy':{'minimum_score':0.9,'minimum_margin':0.05,'minimum_examples':2},'same_speaker_accepted':pdecision['suggested_profile']==pid,'different_speaker_left_unknown':ndecision['suggested_profile'] is None,'positive_scores':[c['score'] for c in pdecision['candidates']],'negative_scores':[c['score'] for c in ndecision['candidates']],'cross_recording_generalization':'unverified','identity_ground_truth':'VoxConverse within-recording reference labels; no personal voice identity inferred'}
(out/'report.json').write_text(json.dumps(report,indent=2));print(json.dumps(report,indent=2))
assert report['same_speaker_accepted'],report
assert report['different_speaker_left_unknown'],report
