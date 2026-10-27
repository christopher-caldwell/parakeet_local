"""Reproducible public LibriSpeech fixture extraction; never extracts archive paths directly."""
import pathlib,hashlib,json,tarfile,collections,subprocess
def digest(path,algorithm='sha256'):
 h=hashlib.new(algorithm)
 with pathlib.Path(path).open('rb') as f:
  for block in iter(lambda:f.read(1024*1024),b''):h.update(block)
 return h.hexdigest()
def prepare_librispeech(root):
 root=pathlib.Path(root)
 archive=root/'dev-clean.tar.gz'
 assert digest(archive,'md5')=='42e2234ba48799c1f50f24a7926300a1','official archive digest mismatch'
 manifest=root/'recognition-fixtures.json'
 if not manifest.exists():
  with tarfile.open(archive) as tar:
   groups=collections.defaultdict(list)
   for m in tar.getmembers():
    if m.isfile() and m.name.endswith('.flac'):groups[m.name.split('/')[-1].split('-')[0]].append(m)
   fixtures=[]
   for speaker in sorted(groups,key=int)[:6]:
    # Interleave chapters to avoid all samples coming from one contiguous passage.
    chapters=collections.defaultdict(list)
    for m in sorted(groups[speaker],key=lambda m:m.name):chapters[m.name.split('/')[-2]].append(m)
    ordered=[]
    while any(chapters.values()):
     for values in chapters.values():
      if values:ordered.append(values.pop(0))
    chosen=[]
    for m in ordered:
     target=root/'recognition-audio'/pathlib.PurePosixPath(m.name).name
     target.parent.mkdir(exist_ok=True)
     if not target.exists():target.write_bytes(tar.extractfile(m).read())
     duration=float(subprocess.check_output(['ffprobe','-v','error','-show_entries','format=duration','-of','default=nw=1:nk=1',str(target)]))
     if duration>=8:
      chosen.append({'path':str(target),'duration_ms':round(duration*1000),'sha256':hashlib.sha256(target.read_bytes()).hexdigest(),'utterance':target.stem})
     if len(chosen)==5:break
    assert len(chosen)==5
    fixtures.append({'speaker':speaker,'known':len(fixtures)<3,'enroll':chosen[:2] if len(fixtures)<3 else [],'tuning':chosen[2:3],'heldout':chosen[3:5]})
   manifest.write_text(json.dumps({'source':'https://www.openslr.org/12/','license':'CC BY 4.0','archive_md5':'42e2234ba48799c1f50f24a7926300a1','split':'dev-clean; disjoint utterances within speaker; not disjoint recording domains','speakers':fixtures},indent=2))
 return json.loads(manifest.read_text())['speakers']
