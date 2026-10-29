"""Exercise the real dispersed compatibility entrypoint in an isolated home."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

REPO = Path(__file__).resolve().parents[2]
FLEET = REPO / ".agents/skills/yggterm-agent-fleet"


class InstalledPorcelain(unittest.TestCase):
    def setUp(self):
        scratch = Path.home()/'.yggterm/scratchpad/porcelain-installed-tests'
        scratch.mkdir(parents=True,exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(dir=scratch)
        self.addCleanup(self.temp.cleanup)
        self.home = Path(self.temp.name)
        self.env = dict(os.environ)
        for key in list(self.env):
            if key.startswith('YGG_') or key.startswith('YGGTERM_'):
                self.env.pop(key)
        self.env.update(HOME=str(self.home),XDG_CONFIG_HOME=str(self.home/'.config'),
                        PATH=str(self.home/'.local/bin')+os.pathsep+os.environ['PATH'],
                        PYTHONDONTWRITEBYTECODE='1')
        owner = self.home/'.local/bin/ygg-memory.py'
        owner.parent.mkdir(parents=True)
        owner.write_text('# owner-managed test sentinel; do not replace\n')
        self.owner = owner
        self.owner_digest = hashlib.sha256(owner.read_bytes()).hexdigest()
        self.install()

    def install(self):
        result = subprocess.run([sys.executable,str(FLEET/'ygg-disperse.py'),'install',
            '--repo',str(FLEET),'--local','--json'],env=self.env,text=True,capture_output=True,timeout=30)
        self.assertEqual(result.returncode,0,result.stderr)
        self.assertGreater(json.loads(result.stdout)['installed'],0)

    def run_ygg(self,*args,code=0):
        result = subprocess.run([str(self.home/'.local/bin/ygg'),*args],env=self.env,
                                text=True,capture_output=True,timeout=15)
        self.assertEqual(result.returncode,code,result.stdout+result.stderr)
        return result.stdout

    def test_actual_install_routes_fs_steer_ci_and_explicit_json(self):
        # Execute the shebang/PATH chain, not an imported function or a direct
        # python invocation that would hide a broken installed launcher.
        before=json.loads(self.run_ygg('fs','status','--json'))
        self.assertFalse(before['initialized'])
        initialized=json.loads(self.run_ygg('fs','init','--json'))
        self.assertTrue(initialized['initialized'])
        seed=json.loads(self.run_ygg('steer','init','--json'))
        self.assertFalse(seed['fleet_stamped'])
        self.assertFalse((self.home/'.codex/AGENTS.md').exists())
        proposal=json.loads(self.run_ygg('steer','plan','codex','--save','--json'))
        self.assertTrue(proposal['changed'])
        receipt=json.loads(self.run_ygg('steer','stamp','codex','--plan',proposal['saved_plan'],'--json'))
        self.assertEqual(receipt['status'],'stamped')
        self.assertEqual(json.loads(self.run_ygg('steer','check','codex','--json'))['status'],'match')
        human=self.run_ygg('ci','status')
        self.assertIn('fleet single build plane',human)
        self.assertNotIn('"watcher_alive"',human)
        self.assertIsInstance(json.loads(self.run_ygg('ci','status','--json')),dict)
        self.assertEqual(json.loads(self.run_ygg('ci','events','--json')),[])
        self.assertIn('no events yet',self.run_ygg('ci','events'))
        self.assertIsInstance(json.loads(self.run_ygg('auth','status','--harness','all','--json')),dict)
        self.assertIn('steer',self.run_ygg('help','steer'))
        examples=json.loads(self.run_ygg('help','examples','--search','failure','--json'))
        self.assertIsInstance(examples['examples'],list)
        missing=json.loads(self.run_ygg('dream','ls','--json',code=3))
        self.assertTrue(any(row['quality']=='unavailable' for row in missing['coverage']))
        self.assertTrue(any(row['code']=='ADAPTER_UNAVAILABLE' for row in missing['findings']))
        self.assertEqual(hashlib.sha256(self.owner.read_bytes()).hexdigest(),self.owner_digest)

    def test_repeat_install_preserves_owner_runner_and_private_policy(self):
        self.run_ygg('fs','init','--json')
        self.run_ygg('steer','init','--json')
        policy=self.home/'.ygg/steer/overlays/policy.json'
        doc=json.loads(policy.read_text());doc['suffix']='Operator-local invented fixture.'
        policy.write_text(json.dumps(doc));expected=policy.read_bytes()
        self.install()
        self.run_ygg('steer','init','--json')
        self.assertEqual(policy.read_bytes(),expected)
        self.assertEqual(hashlib.sha256(self.owner.read_bytes()).hexdigest(),self.owner_digest)
        module=self.home/'.yggterm/bin/ygg/ygg-steer-compiler.py'
        self.assertTrue(module.is_file())
        self.assertFalse(module.is_symlink())


if __name__=='__main__':unittest.main()
