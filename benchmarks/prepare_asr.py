#!/usr/bin/env python3
"""Prepare labeled ASR suites and deterministic non-speech probes; no inference or downloads.
Requires the official OpenSLR dev-clean archive at the root. See README for provenance.
"""
import argparse,json,pathlib,tarfile,wave,struct,random,math
from public_fixtures import prepare_librispeech
parser=argparse.ArgumentParser();parser.add_argument('--root',type=pathlib.Path,default=pathlib.Path('.local/public/librispeech'));parser.add_argument('--noise-root',type=pathlib.Path,default=pathlib.Path('.local/public/non-speech'));args=parser.parse_args()
fixtures=prepare_librispeech(args.root);transcripts={}
with tarfile.open(args.root/'dev-clean.tar.gz') as archive:
 for member in archive:
  if member.isfile() and member.name.endswith('.trans.txt'):
   for line in archive.extractfile(member).read().decode().splitlines():
    key,text=line.split(' ',1);transcripts[key]=text
for split in ['development','heldout']:
 entries=[]
 for speaker in fixtures:
  item=(speaker['enroll'][0] if speaker['enroll'] else speaker['tuning'][0]) if split=='development' else speaker['heldout'][-1]
  entries.append({'id':item['utterance'],'audio':str(pathlib.Path(item['path']).resolve()),'reference':None,'reference_text':transcripts[item['utterance']]})
 suite={'id':'librispeech-asr-'+split,'dataset':'LibriSpeech dev-clean, OpenSLR 12, CC BY 4.0, archive MD5 42e2234ba48799c1f50f24a7926300a1','split':split,'labels':'verified_public','entries':entries,'models':['whisper-base.en','whisper-small.en'],'collar_ms':0,'score_overlap':True,'word_normalization':'english_case_punctuation'}
 (args.root/('asr-'+split+'-suite.json')).write_text(json.dumps(suite,indent=2))
args.noise_root.mkdir(parents=True,exist_ok=True);randomizer=random.Random(20261027);entries=[]
for kind in ['silence','tone','noise']:
 path=args.noise_root/(kind+'.wav')
 with wave.open(str(path),'wb') as audio:
  audio.setparams((1,2,16000,0,'NONE','not compressed'));samples=[]
  for index in range(8*16000):
   value=0 if kind=='silence' else .05*math.sin(2*math.pi*1000*index/16000) if kind=='tone' else max(-1,min(1,randomizer.gauss(0,.02)))
   samples.append(struct.pack('<h',round(value*32767)))
  audio.writeframes(b''.join(samples))
 entries.append({'id':kind,'audio':str(path.resolve()),'reference':None,'reference_text':''})
suite={'id':'non-speech-safeguard-probe','dataset':'Generated silence, 1kHz sine (.05 amplitude), Gaussian noise (.02 std, seed 20261027), mono PCM16 16kHz 8sec','split':'diagnostic','labels':'synthetic','entries':entries,'models':['whisper-base.en','whisper-small.en'],'collar_ms':0,'score_overlap':True,'word_normalization':'english_case_punctuation'}
(args.noise_root/'suite.json').write_text(json.dumps(suite,indent=2));print('Prepared six development, six held-out and three synthetic inputs; no inference or downloads performed.')
