import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

from full_pipeline import FILE_LIMIT, kill_group, read_rttm, supervise, verified_files
from run import digest


@unittest.skipUnless(os.name == 'posix' and hasattr(os, 'wait4'), 'POSIX worker experiment')
class FullPipelineTests(unittest.TestCase):
    def test_rttm_rejects_wrong_file_nonfinite_and_out_of_file(self):
        valid = 'SPEAKER demo 1 0 1 <NA> <NA> one <NA> <NA>'
        self.assertEqual(read_rttm(valid, 'demo', 2)['segments'][0]['speaker'], 'one')
        for invalid in (valid.replace('demo', 'other'), valid.replace('0 1', 'nan 1'),
                        valid.replace('0 1', '0 3'), valid.replace('0 1', '-1 1')):
            with self.assertRaises(ValueError):
                read_rttm(invalid, 'demo', 2)
        self.assertEqual(read_rttm('', 'demo', 2), {'segments': []})

    def test_frozen_files_reject_changed_content_and_escape(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / 'models';root.mkdir()
            model = root / 'weights';model.write_bytes(b'original')
            pins = [{'name': 'weights', 'sha256': digest(model)}]
            verified_files(root, pins)
            model.write_bytes(b'changed')
            with self.assertRaises(ValueError):verified_files(root, pins)
            outside = root.parent / 'outside';outside.write_bytes(b'outside')
            with self.assertRaises(ValueError):
                verified_files(root, [{'name': '../outside', 'sha256': digest(outside)}])

    def test_worker_drops_provider_secrets_and_reports_kernel_peak(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'worker'
            # A harmless canary, never an actual credential.
            os.environ['MINUTES_EVAL_SECRET_CANARY'] = 'must-not-reach-worker'
            try:
                result = supervise([sys.executable, '-c',
                    'import os; assert "MINUTES_EVAL_SECRET_CANARY" not in os.environ; print("ok")'], path)
            finally:os.environ.pop('MINUTES_EVAL_SECRET_CANARY', None)
            self.assertEqual(result['exit'], 0)
            self.assertIsNone(result['reason'])
            self.assertGreater(result['kernel_child_peak_rss_bytes'], 0)
            self.assertEqual(path.with_suffix('.stdout').read_text().strip(), 'ok')

    def test_deadline_and_sampled_memory_guard_terminate_workers(self):
        for kwargs, reason in [({'wall_seconds': 0.15}, 'wall_deadline'),
                               ({'rss_bytes': 1}, 'sampled_group_rss_budget')]:
            with tempfile.TemporaryDirectory() as tmp:
                result = supervise([sys.executable, '-c', 'import time;time.sleep(30)'],
                                   Path(tmp) / 'worker', poll_seconds=0.03, **kwargs)
                self.assertEqual(result['reason'], reason)
                self.assertNotEqual(result['exit'], 0)
                self.assertLess(result['processing_seconds'], 5)

    def test_cancel_kills_descendant_in_owned_process_group(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp);pidfile = root / 'child-pid'
            code = ('import subprocess,time;from pathlib import Path;'
                    f'p=subprocess.Popen([{sys.executable!r},"-c","import time;time.sleep(30)"]);'
                    f'Path({str(pidfile)!r}).write_text(str(p.pid));time.sleep(30)')
            result = supervise([sys.executable, '-c', code], root / 'worker', cancel_after=0.8)
            self.assertEqual(result['reason'], 'cancelled')
            pid = int(pidfile.read_text())
            process = subprocess.run(['ps', '-o', 'stat=', '-p', str(pid)], capture_output=True, text=True)
            self.assertTrue(process.returncode != 0 or not process.stdout.strip()
                            or process.stdout.strip().startswith('Z'), process.stdout)

    def test_denied_cleanup_only_ignores_absent_or_zombie_groups(self):
        with patch('full_pipeline.os.killpg', side_effect=PermissionError), \
                patch('full_pipeline.subprocess.check_output') as status:
            for listing in ('', '123 Z\n', '123 Z+\n456 S\n'):
                status.return_value = listing
                kill_group(123)
            status.return_value = '123 S\n'
            with self.assertRaises(PermissionError):
                kill_group(123)

    def test_kernel_file_limit_stops_runaway_output(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'worker'
            result = supervise([sys.executable, '-c',
                f'import sys;sys.stdout.buffer.write(b"x"*{FILE_LIMIT * 2});sys.stdout.flush()'], path)
            self.assertNotEqual(result['exit'], 0)
            self.assertLessEqual(path.with_suffix('.stdout').stat().st_size, FILE_LIMIT)


if __name__ == '__main__':unittest.main()
