"""Read-only default editor observation; never execution authorization.

The caller is a trusted fleet adapter supplying canonical host_ref and storage
paths, not a worker's claimed host/config. Only an active, verified default
codex/openai consult shape is qualified. Scenarios, fallback, hot-swap and
scope-specific deny routing remain unqualified. No seed activation, repair,
credentials, verifier keys, model calls or writes occur here.

An observation records policy/control identity at read time. It is NOT a launch
token and closes no policy-read-to-launch race. The native controller must
still validate fresh common-fence owner stop, capability, budget, quota,
privacy and actual native account execution before submitting anything.
"""
from dataclasses import dataclass
import json
from pathlib import Path
import stat

from . import swarmconfig as sc
from .review_completion import Admission, _parse, binding_digest


class Hold(Exception):
    """Retain work and policy data; bounded reason code, no fallback."""
    def __init__(self, code):
        self.code = code
        super().__init__(code)


@dataclass(frozen=True)
class Selection:
    editor_identity: str
    editor_binding_digest: str
    config_digest: str
    policy_digest: str
    control_digest: str
    host_ref: str
    account_ref: str
    plan_ref: str
    authority_ref: str
    scenario_policy_ref: str
    # Only immutable bytes/scalars are retained. Mapping properties return
    # detached inspectable JSON copies, never aliases into trusted state.
    _binding_json: bytes
    _policy_json: bytes
    _revision_json: bytes

    @property
    def editor_binding(self):
        return json.loads(self._binding_json)

    @property
    def policy(self):
        return json.loads(self._policy_json)

    @property
    def revision(self):
        return json.loads(self._revision_json)


def _string(value):
    if type(value) is not str or not value.strip():
        raise Hold('missing_or_invalid_reference')
    return value


def _record(space, ref):
    value = space[_string(ref)]
    if type(value) is not dict or not value:
        raise Hold('missing_or_invalid_reference')
    return value


def _optional_file(path):
    try:
        info = path.lstat()
    except FileNotFoundError:
        return None
    if not stat.S_ISREG(info.st_mode) or info.st_size > sc.MAX_BYTES:
        raise Hold('unsafe_control_storage')
    return path.read_bytes()


def _unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise Hold('unknown_deny_schema')
        result[key] = value
    return result


def _controls(state, denials):
    # Any existing legacy override holds, even expired/corrupt. Its writer's
    # docstring is not authentication; this adapter has no override verifier.
    if _optional_file(state / 'editorial' / 'current-editor.json') is not None:
        raise Hold('unqualified_editor_override')
    raw = _optional_file(state / 'control' / 'deny.json')
    if type(denials) is not dict or denials:
        raise Hold('owner_deny_or_unknown')
    deny = None
    if raw is not None:
        deny = json.loads(raw, object_pairs_hook=_unique_object)
        if (type(deny) is not dict or set(deny) != {'latches', 'updated_at'}
                or type(deny['latches']) is not dict
                or type(deny['updated_at']) is not str):
            raise Hold('unknown_deny_schema')
        # Conservatively also hold expired latches: scope/TTL routing is not
        # qualified here. Malformed expiry metadata can never reopen work.
        if deny['latches']:
            raise Hold('owner_deny_or_unknown')
    return binding_digest(dict(deny=deny, override='absent'))


def observe_default_editor(state_dir, live_yaml, host_ref):
    """Fresh canonical load -> immutable Selection or typed Hold.

    authority_ref is an opaque policy handle, not an authenticated principal.
    The full declared binding is preserved with identity=its catalog reference;
    account/plan handles contain no credential values.
    """
    try:
        state = Path(state_dir).expanduser()
        loaded = sc.load(state_dir=state, live_yaml=live_yaml)
        if (type(loaded) is not dict or set(loaded) != {
                'mode', 'config', 'revision', 'source', 'faults', 'denials', 'loaded_at'}
                or loaded['mode'] != 'active' or loaded['faults'] != []
                or loaded['source'] not in ('release', 'checkpoint')):
            raise Hold('unusable_activated_config')
        # Snapshot before validating: loader/caller aliases cannot later mutate
        # the selected projection. Unknown shapes still fail, never empty {}.
        cfg = json.loads(sc.canonical_json(loaded['config']))
        if sc.validate(cfg):
            raise Hold('invalid_activated_config')
        revision = loaded['revision']
        ident = cfg['identity']
        expected = dict(epoch=ident['authority_epoch'], revision=ident['revision'],
                        content_hash=sc.content_hash(cfg), change_id=ident['change_id'])
        if (type(revision) is not dict or set(revision) != set(expected)
                or type(revision.get('epoch')) is not int
                or type(revision.get('revision')) is not int
                or revision != expected):
            raise Hold('config_revision_mismatch')
        control = _controls(state, loaded['denials'])
        host = _string(host_ref)
        if host not in cfg['fleet']['hosts']:
            raise Hold('forbidden_host')
        role = _record(cfg['roles'], 'editor')
        if role['kind'] != 'editorial':
            raise Hold('unsupported_editor_role')
        ref = _string(role['default_binding_ref'])
        cat = cfg['catalog']
        binding = _record(cat['bindings'], ref)
        if binding['status'] != 'admitted':
            raise Hold('binding_not_admitted')
        if role.get('binding_refs') is not None and ref not in role['binding_refs']:
            raise Hold('default_outside_role_bindings')
        scenario_ref = _string(role['scenario_policy_ref'])
        scenario = _record(cfg['editor_scenarios'], scenario_ref)
        authority = _string(role['authority_ref'])
        if (scenario['activation'].get('authority_ref') != authority
                or scenario['activation'].get('requires_admitted_target') is not True
                or ref not in scenario['binding_order']):
            raise Hold('unsupported_scenario_policy')
        harness = _record(cat['harnesses'], binding['harness'])
        provider = _record(cat['providers'], binding['provider'])
        account_ref = _string(binding['account'])
        plan_ref = _string(binding['plan'])
        account = _record(cat['accounts'], account_ref)
        plan = _record(cat['plans'], plan_ref)
        if (account['provider'] != binding['provider']
                or plan['provider'] != binding['provider']
                or plan.get('account') != account_ref):
            raise Hold('funding_reference_mismatch')
        hosts = account.get('permitted_hosts')
        if (type(hosts) is not list or not hosts
                or any(type(h) is not str or h not in cfg['fleet']['hosts'] for h in hosts)
                or host not in hosts):
            raise Hold('forbidden_host')
        caps = binding['capabilities']
        required = role.get('required_capabilities', [])
        if (type(required) is not list or any(type(c) is not str for c in required)
                or 'editorial' not in caps or not set(required).issubset(caps)):
            raise Hold('editor_capability_missing')
        restrictions = binding.get('task_restrictions', [])
        if type(restrictions) not in (list, dict) or restrictions:
            raise Hold('unqualified_task_restrictions')
        if (harness != {'kind': 'consult-cli', 'adapter_ref': 'consult-codex-v1'}
                or provider.get('id') != 'openai'
                or binding['reasoning'] not in sc._ADAPTER_REASONING_SUPPORT['consult-codex-v1']):
            raise Hold('unqualified_native_shape')
        _string(binding['model'])
        editor_binding = dict(identity=ref, **binding)
        projection = dict(role=role, scenario=scenario, harness=harness,
            provider=provider, account=dict(provider=account['provider'], permitted_hosts=hosts),
            plan=plan, host_ref=host, authority_ref=authority)
        return Selection(ref, binding_digest(editor_binding), expected['content_hash'],
            binding_digest(projection), control, host, account_ref, plan_ref,
            authority, scenario_ref, sc.canonical_json(editor_binding),
            sc.canonical_json(projection), sc.canonical_json(expected))
    except Hold:
        raise
    except (OSError, ValueError, TypeError, KeyError, AttributeError, RuntimeError):
        raise Hold('policy_observation_unresolved') from None


def check_admitted_editor(admission, state_dir, live_yaml, host_ref):
    """Compare the sealed Admission with a new load; never rewrite admission.

    Original criteria, worker and physical dispatch identity remain untouched.
    Caller owns authentication of the sealed Admission envelope.
    """
    try:
        admitted = _parse(Admission, admission)
    except (ValueError, TypeError, KeyError, AttributeError):
        raise Hold('malformed_admission') from None
    selected = observe_default_editor(state_dir, live_yaml, host_ref)
    if any(getattr(admitted, field) != getattr(selected, field) for field in (
            'editor_identity', 'editor_binding_digest', 'config_digest')):
        raise Hold('admitted_policy_mismatch')
    return selected
