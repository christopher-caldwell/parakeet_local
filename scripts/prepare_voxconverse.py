#!/usr/bin/env python3
"""Fetch a small deterministic CC BY 4.0 VoxConverse development subset using ZIP ranges.
No private data is read or transmitted. Requires curl, official annotations clone, and FFmpeg.
"""
import argparse,io,json,pathlib,re,subprocess,zipfile,hashlib
URL='https://www.robots.ox.ac.uk/~vgg/data/voxconverse/data/voxconverse_dev_wav.zip'
def curl(*args):return subprocess.check_output(['curl','--fail','--silent','--show-error','--location','--proto','=https','--max-time','180',*args,URL])
class RemoteZip(io.RawIOBase):
 def __init__(self):
  headers=curl('--head').decode(); self.length=int(re.findall(r'(?im)^content-length:\s*(\d+)',headers)[-1]);self.pos=0
 def seekable(self):return True
 def seek(self,offset,whence=0):self.pos=offset+(self.pos if whence==1 else self.length if whence==2 else 0);return self.pos
 def tell(self):return self.pos
 def read(self,size=-1):
  if size<0:size=self.length-self.pos
  size=min(size,self.length-self.pos)
  if not size:return b''
  if size>128*1024*1024:raise RuntimeError('refusing unexpectedly large ZIP range')
  data=curl('--range',f'{self.pos}-{self.pos+size-1}','--write-out','\n%{http_code}')
  body,status=data.rsplit(b'\n',1)
  if status!=b'206':raise RuntimeError('server did not honor range request')
  if len(body)!=size:raise RuntimeError('truncated range')
  self.pos+=size;return body
parser=argparse.ArgumentParser();parser.add_argument('--annotations',type=pathlib.Path,default=pathlib.Path('.references/voxconverse'));parser.add_argument('--output',type=pathlib.Path,default=pathlib.Path('.local/public/voxconverse'));parser.add_argument('--split',choices=['dev','test'],default='dev');parser.add_argument('--count',type=int,default=3);args=parser.parse_args();URL=f'https://www.robots.ox.ac.uk/~vgg/data/voxconverse/data/voxconverse_{args.split}_wav.zip';args.output.mkdir(parents=True,exist_ok=True)
candidates=[]
for p in sorted((args.annotations/args.split).glob('*.rttm')):
 turns=[]
 for line in p.read_text().splitlines():
  t=line.split();start=round(float(t[3])*1000);end=start+round(float(t[4])*1000);turns.append({'start_ms':start,'end_ms':end,'speaker':t[7],'track_id':None})
 if not turns:continue
 duration=max(t['end_ms'] for t in turns);speakers={t['speaker'] for t in turns}
 if 20000<=duration<=120000 and len(speakers)>=2:candidates.append((duration,p,turns))
chosen=sorted(candidates,key=lambda x:(x[0],x[1].stem))[:args.count]
z=zipfile.ZipFile(RemoteZip());names=z.namelist();entries=[]
for duration,p,turns in chosen:
 name=next(n for n in names if n.endswith('/'+p.stem+'.wav') or n==p.stem+'.wav');out=args.output/(p.stem+'.wav')
 if not out.exists():out.write_bytes(z.read(name))
 ref=args.output/(p.stem+'.reference.json');ref.write_text(json.dumps(turns,indent=2));entries.append({'id':p.stem,'audio':str(out.resolve()),'reference':str(ref.resolve()),'reference_text':None});print(p.stem,'duration_ms',duration,'speakers',len({t['speaker'] for t in turns}),flush=True)
suite={'id':f'voxconverse-{args.split}-short{args.count}','dataset':'VoxConverse 0.3 CC BY 4.0 (Chung et al., Interspeech 2020)','split':'held-out-test' if args.split=='test' else 'development-diagnostic','labels':'verified_public','entries':entries,'models':['speakrs-community-1-coreml'],'collar_ms':0,'score_overlap':True}
(args.output/'suite.json').write_text(json.dumps(suite,indent=2));print('suite:',args.output/'suite.json')
