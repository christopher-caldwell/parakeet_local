#!/usr/bin/env python3
"""Public-label, disjoint-utterance recognition probe; private local outputs only.
LibriSpeech dev-clean (OpenSLR 12), CC BY 4.0. Tune on designated utterances,
then score untouched utterances. This clean read-speech probe is not meeting accuracy.
"""
import argparse, hashlib, json, pathlib, subprocess, tarfile, collections
p=argparse.ArgumentParser();p.add_argument('--backend',choices=['coreml','cpu'],default='coreml');args=p.parse_args()
from public_fixtures import prepare_librispeech
root=pathlib.Path('.local/public/librispeech');
fixtures=prepare_librispeech(root);out=root/f'recognition-{args.backend}';out.mkdir(exist_ok=True)
# Independent APFS model clones keep benchmark selection out of personal settings.
runtime=out/'runtime';models=runtime/'models';models.mkdir(parents=True,exist_ok=True)
model_id='speakrs-community-1-'+args.backend
if not (models/model_id).exists():subprocess.run(['/bin/cp','-cRp',str(pathlib.Path('.local/app/models')/model_id),str(models/model_id)],check=True)
db=out/'app.db';base=['target/debug/parakeet','--data-root',str(runtime),'--database',str(db)]
def run(*cmd):
 proc=subprocess.run([*base,*cmd],capture_output=True);result=json.loads(proc.stdout)
 if proc.returncode:raise RuntimeError(result['error'])
 return result['data']
if not db.exists():run('db','create')
run('db','migrate');run('models','select-embedding','speakrs-community-1-'+args.backend)
def interval(item):
 result=run('meetings','import',item['path']);rid=result['recordings'][0];track=run('meetings','show',rid)['tracks'][0]['id']
 return rid,track,1000,min(item['duration_ms']-500,11000)
profiles={p['name']:p['id'] for p in run('speakers','list')}
for f in fixtures:
 if not f['known']:continue
 name='LibriSpeech fixture '+f['speaker'];pid=profiles.get(name) or run('speakers','create',name)['id'];profiles[name]=pid
 examples=run('speakers','examples',pid)
 for item in f['enroll']:
  rid,track,a,b=interval(item)
  if not any(e['recording_id']==rid for e in examples):run('speakers','enroll',pid,rid,'--track',track,'--start-ms',str(a),'--end-ms',str(b),'--confirmed')
def collect(split):
 output=[]
 for f in fixtures:
  for item in f[split]:
   rid,track,a,b=interval(item)
   d=run('speakers','suggest',rid,'--track',track,'--start-ms',str(a),'--end-ms',str(b),'--minimum-score','1','--minimum-margin','0.05','--minimum-examples','2')
   d=d.get('decision',d)
   output.append({'utterance':item['utterance'],'expected_profile':profiles['LibriSpeech fixture '+f['speaker']] if f['known'] else None,'candidates':d['candidates']})
 return output
def score(rows,threshold):
 totals={'known':0,'unknown':0,'accepted_correct':0,'false_acceptances':0,'missed_known':0}
 for row in rows:
  c=row['candidates'];best=c[0] if c else None;chosen=best['profile_id'] if best and best['score']>=threshold and best['examples']>=2 and best['score']-(c[1]['score'] if len(c)>1 else -1)>=0.05 else None
  expected=row['expected_profile'];totals['known' if expected else 'unknown']+=1
  if chosen:
   totals['accepted_correct' if chosen==expected else 'false_acceptances']+=1
  if expected and chosen!=expected:totals['missed_known']+=1
 return totals
tuning=collect('tuning');(out/'tuning-scores.json').write_text(json.dumps(tuning,indent=2))
trials=[(t,score(tuning,t)) for t in [.55,.6,.65,.7,.75,.8,.85,.9,.95]]
safe=[(t,s) for t,s in trials if s['false_acceptances']==0]
threshold=max(safe,key=lambda x:(x[1]['accepted_correct'],x[0]))[0] if safe else 1.
policy={'minimum_score':threshold,'minimum_margin':.05,'minimum_examples':2,'selected_using':'tuning utterances only; maximize correct acceptances with zero observed false accepts, highest threshold tie break','tuning_trials':trials}
(out/'policy-before-heldout.json').write_text(json.dumps(policy,indent=2))
heldout=collect('heldout');(out/'heldout-scores.json').write_text(json.dumps(heldout,indent=2))
report={'dataset':'LibriSpeech dev-clean','backend':args.backend,'preprocessing':'masked-window-v1','protocol':'3 known speakers, 2 disjoint enrollment utterances per speaker, 1 tuning utterance per speaker, 2 held-out utterances per speaker; 3 unfamiliar speakers with disjoint tuning and held-out utterances','policy':policy,'tuning':score(tuning,threshold),'heldout':score(heldout,threshold),'limitations':['Small clean read-speech probe; not personal meeting accuracy','Speaker identities are dataset IDs','No cross-device/domain validation','Threshold not enabled globally']}
(out/'report.json').write_text(json.dumps(report,indent=2));print(json.dumps(report,indent=2))
