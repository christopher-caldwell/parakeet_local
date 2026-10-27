#!/usr/bin/env python3
"""Local macOS observations; all UI interaction stays with the real native app.

Adapted accounting approach: Smaller Talk's scripts/memory_bench/macos.py (MIT).
No provider execution, window/input automation, database writes or uploads.
"""
import argparse
import ctypes
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import signal
import subprocess
import time


class RUsage(ctypes.Structure):
    _fields_ = [("uuid", ctypes.c_ubyte * 16)] + [(name, ctypes.c_uint64) for name in (
        "user_time", "system_time", "pkg_idle_wkups", "interrupt_wkups", "pageins",
        "wired_size", "resident_size", "phys_footprint", "proc_start_abstime", "proc_exit_abstime")]


class Accounting:
    def __init__(self):
        self.lib = ctypes.CDLL('/usr/lib/libproc.dylib', use_errno=True)
        self.call = self.lib.proc_pid_rusage
        self.call.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_void_p]
        self.call.restype = ctypes.c_int

    def read(self, pid):
        value = RUsage()
        if self.call(pid, 0, ctypes.byref(value)) != 0:
            code = ctypes.get_errno()
            raise OSError(code, os.strerror(code))
        return dict(rss_bytes=value.resident_size, footprint_bytes=value.phys_footprint,
                    cpu_ns=value.user_time + value.system_time, identity=value.proc_start_abstime)


def write_json(path, data):
    temporary = path.with_suffix(path.suffix + '.new')
    with temporary.open('x') as stream:
        os.chmod(temporary, 0o600)
        json.dump(data, stream, indent=2)
    temporary.replace(path)


def append(path, value):
    with path.open('a') as stream:
        os.chmod(path, 0o600)
        stream.write(json.dumps(value) + '\n')


def processes():
    result = subprocess.run(['/bin/ps', '-ww', '-axo', 'pid=,ppid=,comm='],
                            check=True, capture_output=True, text=True, timeout=5)
    return {int(fields[0]): (int(fields[1]), fields[2]) for line in result.stdout.splitlines()
            if len(fields := line.split(None, 2)) == 3}


def descendants(table, root):
    members = {root}
    while True:
        found = {pid for pid, (parent, _) in table.items() if parent in members}
        if found <= members:
            return members
        members |= found


def capture(args):
    if platform.system() != 'Darwin':
        raise RuntimeError('resource accounting requires macOS')
    root = args.out.resolve()
    if not (root / 'benchmark.json').is_file():
        raise ValueError('run prepare first; an explicit new output directory is required')
    if (root / 'samples.jsonl').exists():
        raise ValueError('capture already exists; use a new output directory')
    accounting = Accounting()
    identity = accounting.read(args.pid)['identity']
    executable = processes()[args.pid][1]
    if Path(executable).resolve() != args.binary.resolve():
        raise ValueError('selected binary does not match the observed root executable')
    manifest = dict(protocol=1, started_at_unix_s=time.time(), pid=args.pid,
                    process_identity=identity, binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest(),
                    executable=str(args.binary.resolve()), label=args.label,
                    interval_s=args.interval, host=platform.platform(), cpu_count=os.cpu_count(),
                    load_average=os.getloadavg(), data_root=str(args.data_root.resolve()),
                    source_revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
                    dirty=bool(subprocess.check_output(['git', 'status', '--porcelain'], text=True)),
                    limitations=['Sampled maxima are not allocation peaks.',
                        'Footprint sums are process accounting, not deduplicated physical memory.',
                        'CPU uses interval deltas of libproc user/system nanoseconds; 100% is one logical CPU.',
                        'GPU/ANE utilization and device allocations are not fully observed.',
                        'Short-lived descendants can be missed; read failures stay missing.',
                        'Frame timings are instrumented GPUI CPU/layout/submission work, not display pixel validation.',
                        'Input histogram phase boundaries have up to one observation interval uncertainty.'])
    write_json(root / 'capture.json', manifest)
    previous, seen = {}, {args.pid: identity}
    samples, interrupted = 0, False
    def stop(*_):
        nonlocal interrupted
        interrupted = True
    signal.signal(signal.SIGINT, stop)
    signal.signal(signal.SIGTERM, stop)
    next_tick = time.monotonic()
    while not interrupted and not (root / 'stop').exists():
        started, now = time.monotonic(), time.time()
        table = processes()
        rows, failures = [], []
        members = descendants(table, args.pid)
        members |= {pid for pid in seen if pid in table}  # retain observed reparented children, identity checked below
        for pid in sorted(members):
            try:
                usage = accounting.read(pid)
                if pid in seen and seen[pid] != usage['identity']:
                    if pid == args.pid:
                        raise RuntimeError('root PID identity changed')
                    continue
                seen[pid] = usage['identity']
                old = previous.get(pid)
                cpu = None
                if old and old[0] == usage['identity'] and started > old[2] and usage['cpu_ns'] >= old[1]:
                    cpu = (usage['cpu_ns'] - old[1]) / 1e9 / (started - old[2]) * 100
                previous[pid] = (usage['identity'], usage['cpu_ns'], started)
                rows.append(dict(pid=pid, executable=table.get(pid, (0, 'exited'))[1], **usage, cpu_percent=cpu))
            except OSError as error:
                failures.append(dict(pid=pid, error=str(error)))
        sample = dict(at_unix_s=now, monotonic_s=started, processes=rows, failures=failures,
                      duration_s=time.monotonic() - started, host_load_1m=os.getloadavg()[0])
        append(root / 'samples.jsonl', sample)
        samples += 1
        if not any(row['pid'] == args.pid for row in rows):
            manifest['incomplete_reason'] = 'root process exited during capture'
            break
        next_tick += args.interval
        time.sleep(max(0, next_tick - time.monotonic()))
    manifest.update(finished_at_unix_s=time.time(), samples=samples, interrupted=interrupted)
    write_json(root / 'capture-finished.json', manifest)


def quantiles(values):
    values = sorted(v for v in values if v is not None and math.isfinite(v))
    return {key: (values[min(len(values)-1, max(0, math.ceil(len(values)*q)-1))] if values else None)
            for key, q in [('p50', .5), ('p95', .95), ('max', 1)]} | {'samples': len(values)}


def read_lines(path):
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text().splitlines() if line.strip()]


def input_delta(before, after):
    old = dict(before)
    buckets = [(value, count - old.get(value, 0)) for value, count in after]
    current = dict(after)
    if any(current.get(value, 0) < count for value, count in old.items()):
        raise ValueError('input histogram reset; phase cannot be compared')
    total = sum(count for _, count in buckets)
    def at(q):
        running = 0
        for value, count in sorted(buckets):
            running += count
            if count and running >= max(1, math.ceil(total*q)):
                return value / 1e6
        return None
    return dict(p50=at(.5), p95=at(.95), max=at(1), samples=total)


def report(root):
    capture = json.loads((root / 'capture-finished.json').read_text())
    phases = read_lines(root / 'phase-events.jsonl')
    samples = read_lines(root / 'samples.jsonl')
    events = read_lines(root / 'app-events.jsonl')
    frames = [event for event in events if event['event'] == 'frame']
    observations = [event for event in events if event['event'] == 'ui']
    summaries = []
    for index, phase in enumerate(phases):
        start = phase['at_unix_s']
        end = phases[index+1]['at_unix_s'] if index+1 < len(phases) else capture['finished_at_unix_s']
        within = lambda row: start <= row['at_unix_s'] < end
        resource = [sample for sample in samples if within(sample)]
        drawn = [event['data'] for event in frames if within(event)]
        ui = [event['data'] for event in observations if within(event)]
        complete = [sample for sample in resource if not sample['failures'] and sample['processes']]
        ready_cpu = [sample for sample in complete if all(p['cpu_percent'] is not None for p in sample['processes'])]
        before = [event for event in observations if event['at_unix_s'] <= start]
        after = [event for event in observations if event['at_unix_s'] <= end]
        latency = input_delta(before[-1]['data']['input_latency_buckets_ns'] if before else [],
                              after[-1]['data']['input_latency_buckets_ns'] if after else [])
        measured = [frame for frame in drawn if frame['late'] is not None]
        issues = []
        if len(complete) < 3:
            issues.append('fewer than three complete resource samples')
        if phase['kind'] == 'scroll' and (len(drawn) < 10 or len({row['scroll_y'] for row in ui}) < 3):
            issues.append('insufficient drawn frames or observed scroll movement')
        summaries.append(dict(name=phase['name'], kind=phase['kind'], duration_s=end-start,
            validity='invalid' if issues else 'observed', issues=issues,
            footprint_mib=quantiles(sum(p['footprint_bytes'] for p in sample['processes'])/2**20 for sample in complete),
            rss_mib=quantiles(sum(p['rss_bytes'] for p in sample['processes'])/2**20 for sample in complete),
            cpu_percent=quantiles(sum(p['cpu_percent'] for p in sample['processes']) for sample in ready_cpu),
            sampling_gaps_s=quantiles(b['monotonic_s']-a['monotonic_s'] for a,b in zip(resource, resource[1:])),
            resource_failures=sum(len(sample['failures']) for sample in resource),
            host_load_1m=quantiles(sample['host_load_1m'] for sample in resource),
            frames=len(drawn), late_frames=sum(frame['late'] for frame in measured), measured_frames=len(measured),
            frame_draw_ms=quantiles(frame['draw_ms'] for frame in drawn),
            frame_work_ms=quantiles(frame['work_ms'] for frame in drawn),
            prepaint_ms=quantiles(frame['prepaint_ms'] for frame in drawn),
            request_layout_ms=quantiles(frame['request_layout_ms'] for frame in drawn),
            dirty_to_present_ms=quantiles(frame['dirty_to_present_ms'] for frame in drawn),
            input_to_frame_ms=latency,
            segments=sorted({row['segments'] for row in ui if row['segments'] is not None}),
            scroll_y_range=[min((row['scroll_y'] for row in ui),default=None),max((row['scroll_y'] for row in ui),default=None)],
            pending_range=[min((row['pending'] for row in ui),default=None),max((row['pending'] for row in ui),default=None)]))
    finished = [event['data'] for event in events if event['event'] == 'finished']
    issues = []
    if not finished:
        issues.append('observer did not record a normal finish; evidence may be incomplete')
    elif finished[-1]['dropped_ui_samples']:
        issues.append('observer dropped UI samples')
    if capture.get('incomplete_reason') or capture.get('interrupted'):
        issues.append('resource capture ended unexpectedly')
    summary = dict(protocol=1, capture=capture, phases=summaries,
                   validity='invalid' if issues else 'observed', issues=issues,
                   observer_finished=bool(finished), dropped_ui_samples=finished[-1]['dropped_ui_samples'] if finished else None)
    write_json(root / 'report.json', summary)
    print(json.dumps(summary, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    actions = parser.add_subparsers(dest='command', required=True)
    prepare = actions.add_parser('prepare'); prepare.add_argument('--out', type=Path, required=True)
    record = actions.add_parser('record')
    for name in ('out', 'binary', 'data-root'):
        record.add_argument('--'+name, type=Path, required=True)
    record.add_argument('--pid', type=int, required=True)
    record.add_argument('--label', required=True)
    record.add_argument('--interval', type=float, default=.2)
    phase = actions.add_parser('phase'); phase.add_argument('--out', type=Path, required=True)
    phase.add_argument('--kind', choices=['idle','open','scroll','find','inference','closed'], required=True)
    phase.add_argument('name')
    for command in ('stop','report'):
        sub = actions.add_parser(command); sub.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    if args.command == 'prepare':
        args.out.mkdir(mode=0o700, parents=True, exist_ok=False)
        write_json(args.out/'benchmark.json', dict(protocol=1))
    elif args.command == 'record':
        if not .05 <= args.interval <= 10:
            parser.error('interval must be 0.05–10 seconds')
        capture(args)
    elif args.command == 'phase':
        append(args.out/'phase-events.jsonl', dict(at_unix_s=time.time(), name=args.name, kind=args.kind))
    elif args.command == 'stop':
        (args.out/'stop').touch(exist_ok=False)
    else:
        report(args.out)


if __name__ == '__main__':
    main()
