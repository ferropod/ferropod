#!/usr/bin/env python3
"""Deploy a CI-built Wasm component using a runtime-scoped Ferropod token."""
import argparse
import json
import os
from pathlib import Path
import secrets
import sys
from urllib.error import HTTPError, URLError
from urllib.parse import quote, urlsplit
from urllib.request import HTTPRedirectHandler, Request, build_opener


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--url', required=True, help='Ferropod origin, including https://')
    parser.add_argument('--runtime', required=True, help='Runtime ID or name')
    parser.add_argument('--file', required=True, type=Path, help='Built Wasm component')
    parser.add_argument('--target', choices=['prod', 'preview', 'ready'], default='preview')
    args = parser.parse_args()
    token = os.environ.get('FERROPOD_TOKEN', '')
    if not token:
        parser.error('FERROPOD_TOKEN is required')
    url = urlsplit(args.url)
    if (url.scheme not in ('http', 'https') or not url.hostname or url.username or url.password
            or url.path not in ('', '/') or url.query or url.fragment):
        parser.error('--url must be an HTTP(S) origin without credentials or a path')
    if url.scheme != 'https' and url.hostname not in ('127.0.0.1', 'localhost', '::1'):
        parser.error('HTTPS is required except for local testing')
    if args.file.suffix.lower() not in ('.wasm', '.wat'):
        parser.error('--file must be a .wasm or .wat component')
    size = args.file.stat().st_size
    if size == 0 or size > 64 * 1024 * 1024:
        parser.error('--file must be non-empty and at most 64 MiB')
    origin = args.url.rstrip('/')
    runtime_path = '/_api/v1/runtimes/' + quote(args.runtime, safe='')
    opener = build_opener(NoRedirect())

    def api(path, method='GET', data=None, content_type=None):
        headers = {'Authorization': 'Bearer ' + token}
        if content_type:
            headers['Content-Type'] = content_type
        req = Request(origin + path, data=data, headers=headers, method=method)
        try:
            with opener.open(req, timeout=180) as response:
                return json.load(response)
        except HTTPError as error:
            detail = error.read(4096).decode('utf-8', 'replace')
            try:
                detail = json.loads(detail).get('error', detail)
            except (ValueError, AttributeError):
                pass
            raise RuntimeError(f'HTTP {error.code}: {detail}') from None
        except URLError as error:
            raise RuntimeError(f'connection failed: {error.reason}') from None

    runtime = api(runtime_path)
    deployments = api(runtime_path + '/deployments')
    base_id = (runtime.get('preview_deployment_id') if args.target == 'preview' else None) or runtime.get('active_deployment_id')
    base = next((item for item in deployments if item['id'] == base_id), {})
    if base_id and not base:
        raise RuntimeError('current deployment was not returned; refusing to discard its settings')
    artifact = api(runtime_path + '/artifacts/ci-' + secrets.token_hex(16) + '.wasm',
                   'PUT', args.file.read_bytes(), 'application/octet-stream')
    payload = {key: base[key] for key in ('assets_bucket', 'assets_prefix', 'spa_fallback', 'bindings', 'crons') if key in base}
    payload.update(artifact_path=artifact['artifact_path'], target_env=None if args.target == 'ready' else args.target)
    result = api(runtime_path + '/deployments', 'POST', json.dumps(payload).encode(), 'application/json')
    print('Deployment ' + result['id'] + ' created (' + args.target + ')')


if __name__ == '__main__':
    try:
        main()
    except (OSError, RuntimeError, ValueError) as error:
        print(f'deploy failed: {error}', file=sys.stderr)
        sys.exit(1)
