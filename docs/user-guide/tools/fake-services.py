#!/usr/bin/env python3
"""fake-services.py [--port N]

A small stand-in for GitLab, Jira, GitHub and Slack, for the user guide's screenshots
(capture-all.sh starts it, points radar's config at it, and stops it). Standard library only.

It listens on 127.0.0.1 (a free port unless --port is given), prints its origin
(http://127.0.0.1:PORT) as its first line on stdout, and answers the requests radar's
clients send, from one invented organisation (acme: alice, bob, carol):

  GitLab   POST /api/graphql (any query: it is parsed and answered from the sample data),
           GET /api/v4/projects/<path>/merge_requests/<iid>/diffs, .../jobs/<id>/trace
  Jira     GET /rest/api/3/... (myself, search/jql, issue, comment, remotelink, transitions,
           field, project) and /rest/dev-status/latest/issue/...
  GitHub   GET /user, /rate_limit, /repos/<o>/<r>[/pulls[/<n>]|/commits/<sha>/check-runs|statuses],
           POST /graphql
  Slack    GET|POST /api/<method> (auth.test, users.info, usergroups.list, conversations.*,
           users.conversations, search.messages, chat.postMessage, reactions.add)

Everything is read-only: a write is answered as if it worked and nothing is kept. A request
it does not know is answered 404 and logged to stderr as "UNKNOWN ...", so gaps show.
Times are relative to the moment the server starts, so the data always looks recent.
"""
import hashlib
import json
import re
import sys
import time
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

NOW = int(time.time())


def iso(ago=0):
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(NOW - ago))


def jira_time(ago=0):
    return time.strftime("%Y-%m-%dT%H:%M:%S.000+0000", time.gmtime(NOW - ago))


MIN, HOUR, DAY = 60, 3600, 86400


def log(*a):
    print(*a, file=sys.stderr, flush=True)


# ---------------------------------------------------------------- a small GraphQL engine
_TOK = re.compile(
    r'(?:\s+|,|#[^\n]*)*(?:(?P<p>\.\.\.|[{}()\[\]:!$=@&|])|(?P<n>-?\d+(?:\.\d+)?)'
    r'|(?P<s>"(?:[^"\\]|\\.)*")|(?P<w>[A-Za-z_][A-Za-z0-9_]*))'
)


def _tokens(src):
    out, pos = [], 0
    while True:
        m = _TOK.match(src, pos)
        if not m:
            break
        pos = m.end()
        for kind in "pnsw":
            if m.group(kind) is not None:
                out.append((kind, m.group(kind)))
                break
    return out


class _Parser:
    def __init__(self, src):
        self.t = _tokens(src)
        self.i = 0

    def peek(self, v=None):
        if self.i >= len(self.t):
            return False if v is not None else None
        return self.t[self.i][1] == v if v is not None else self.t[self.i]

    def take(self):
        tok = self.t[self.i]
        self.i += 1
        return tok

    def skip_group(self):  # at "(": skip to its matching ")"
        depth = 0
        while True:
            v = self.take()[1]
            depth += (v == "(") - (v == ")")
            if depth == 0:
                return

    def document(self):
        if self.peek("query") or self.peek("mutation"):
            self.take()
            if self.peek() and self.peek()[0] == "w":
                self.take()
            if self.peek("("):
                self.skip_group()
        return self.selection()

    def selection(self):
        out = []
        self.take()  # {
        while not self.peek("}"):
            if self.peek("..."):
                self.take()
                if self.peek("on"):
                    self.take()
                    self.take()
                out.append({"frag": self.selection()})
                continue
            name = self.take()[1]
            alias = None
            if self.peek(":"):
                self.take()
                alias, name = name, self.take()[1]
            args = {}
            if self.peek("("):
                self.take()
                while not self.peek(")"):
                    k = self.take()[1]
                    self.take()  # :
                    args[k] = self.value()
                self.take()
            while self.peek("@"):  # directives: ignored
                self.take()
                self.take()
                if self.peek("("):
                    self.skip_group()
            sub = self.selection() if self.peek("{") else None
            out.append({"name": name, "alias": alias or name, "args": args, "sub": sub})
        self.take()  # }
        return out

    def value(self):
        kind, v = self.take()
        if v == "$":
            return ("var", self.take()[1])
        if kind == "s":
            return json.loads(v)
        if kind == "n":
            return float(v) if "." in v else int(v)
        if v == "[":
            xs = []
            while not self.peek("]"):
                xs.append(self.value())
            self.take()
            return xs
        if v == "{":
            d = {}
            while not self.peek("}"):
                k = self.take()[1]
                self.take()
                d[k] = self.value()
            self.take()
            return d
        return {"true": True, "false": False, "null": None}.get(v, v)


def _resolve(v, variables):
    if isinstance(v, tuple) and v and v[0] == "var":
        return variables.get(v[1])
    if isinstance(v, list):
        return [_resolve(x, variables) for x in v]
    if isinstance(v, dict):
        return {k: _resolve(x, variables) for k, x in v.items()}
    return v


_missing = set()


def _execute(sels, node, variables, where):
    out = {}
    for f in sels:
        if "frag" in f:
            out.update(_execute(f["frag"], node, variables, where))
            continue
        name = f["name"]
        if name == "__typename":
            out[f["alias"]] = (node or {}).get("__typename", where)
            continue
        if not isinstance(node, dict) or name not in node:
            if (where, name) not in _missing:
                _missing.add((where, name))
                log("graphql: no field %s on %s (answered null)" % (name, where))
            out[f["alias"]] = None
            continue
        val = node[name]
        if callable(val):
            val = val({k: _resolve(a, variables) for k, a in f["args"].items()})
        out[f["alias"]] = _complete(val, f["sub"], variables, name)
    return out


def _complete(val, sub, variables, where):
    if val is None or sub is None:
        return val
    if isinstance(val, list):
        return [_complete(x, sub, variables, where) for x in val]
    if isinstance(val, dict):
        return _execute(sub, val, variables, where)
    return val


def graphql(query, variables, root):
    return {"data": _execute(_Parser(query).document(), root, variables or {}, "Query")}


def connection(items, args=None):
    items = list(items)
    first = (args or {}).get("first")
    if isinstance(first, int):
        items = items[:first]
    return {"nodes": items, "count": len(items), "totalCount": len(items),
            "pageInfo": {"hasNextPage": False, "endCursor": None}}


# ---------------------------------------------------------------- GitLab
GL_BASE = "https://gitlab.example"
PEOPLE = {
    "alice": "Alice Example",
    "bob": "Bob Builder",
    "carol": "Carol Coder",
}
GL_ME = "alice"


def person(u):
    return {"username": u, "name": PEOPLE[u]}


def sha(n):
    return hashlib.sha1(("acme-%d" % n).encode()).hexdigest()


GL_PROJECTS = {}


def gl_project(path, last_title, last_author, ago, main_status, pipeline_id):
    name = path.rsplit("/", 1)[1]
    ci = {
        "id": "gid://gitlab/Ci::Pipeline/%d" % pipeline_id,
        "sha": sha(pipeline_id), "ref": "main", "status": main_status,
        "path": "/%s/-/pipelines/%d" % (path, pipeline_id), "updatedAt": iso(ago),
    }
    GL_PROJECTS[path] = {
        "name": name, "fullPath": path, "archived": False, "lastActivityAt": iso(ago),
        "repository": {"rootRef": "main", "tree": {"lastCommit": {
            "sha": sha(pipeline_id), "shortId": sha(pipeline_id)[:8], "authorName": PEOPLE[last_author],
            "authoredDate": iso(ago), "title": last_title}}},
        "_pipelines": [ci],
    }


gl_project("acme/team-a/api", "Merge branch 'fix-pagination' into 'main'", "carol", 2 * DAY, "SUCCESS", 501)
gl_project("acme/team-a/web", "Bump the header height", "bob", 3 * HOUR, "SUCCESS", 311)
gl_project("acme/team-a/docs", "Explain how to run the tests", "alice", 5 * DAY, "SUCCESS", 77)

STAGES = [("build", "success"), ("test", "success"), ("deploy", "skipped")]


def gl_pipeline(status, path, pid, duration=187):
    states = {"SUCCESS": ["success", "success", "success"],
              "FAILED": ["success", "failed", "skipped"],
              "RUNNING": ["success", "running", "created"]}[status]
    return {
        "iid": str(pid), "status": status, "path": "%s/-/pipelines/%d" % (path, pid), "duration": duration,
        "stages": {"nodes": [{"name": n, "status": s, "jobs": {"nodes": []}}
                             for (n, _), s in zip(STAGES, states)]},
    }


def gl_jobs(pipe, project_path, base_id):
    jobs = {"build": [("compile", 62)], "test": [("unit", 71), ("lint", 28)], "deploy": [("review-app", 0)]}
    for stage in pipe["stages"]["nodes"]:
        nodes = []
        for i, (name, dur) in enumerate(jobs[stage["name"]]):
            status = {"success": "SUCCESS", "failed": "FAILED", "running": "RUNNING"}.get(stage["status"], "CREATED")
            if stage["status"] == "skipped":
                status = "SKIPPED"
            nodes.append({"name": name, "status": status, "duration": dur if status != "SKIPPED" else None,
                          "webPath": "/%s/-/jobs/%d" % (project_path, base_id + len(pipe["_seen"])),
                          "allowFailure": False})
            pipe["_seen"].append(1)
        stage["jobs"] = {"nodes": nodes}
    return pipe


def mr(project, iid, title, author, src, ago, **kw):
    path = "acme/team-a/" + project
    status = kw.get("pipeline", "SUCCESS")
    pid = 600 + iid
    pipe = gl_pipeline(status, "/" + path, pid) if status else None
    if pipe:
        pipe["_seen"] = []
        gl_jobs(pipe, path, 9000 + iid * 10)
    checks = kw.get("checks", [("CI_MUST_PASS", "SUCCESS")])
    node = {
        "iid": str(iid), "title": title, "draft": kw.get("draft", False),
        "webUrl": "%s/%s/-/merge_requests/%d" % (GL_BASE, path, iid), "updatedAt": iso(ago),
        "sourceBranch": src, "targetBranch": "main", "diffHeadSha": sha(iid),
        "userNotesCount": kw.get("notes", 0), "conflicts": False,
        "detailedMergeStatus": kw.get("merge", "NOT_APPROVED"),
        "approved": kw.get("approved", False), "approvalsRequired": 1,
        "approvalsLeft": 0 if kw.get("approved") else 1, "shouldBeRebased": False,
        "mergeabilityChecks": [{"identifier": i, "status": s} for i, s in checks],
        "author": person(author),
        "reviewers": {"nodes": [{"username": u} for u in kw.get("reviewers", [])]},
        "approvedBy": {"nodes": [{"username": u} for u in kw.get("approved_by", [])]},
        "headPipeline": pipe,
        "resolvableDiscussionsCount": kw.get("threads", 0), "resolvedDiscussionsCount": kw.get("resolved", 0),
        "notes": {"nodes": kw.get("last", [])},
        "diffStatsSummary": {"additions": kw.get("add", 24), "deletions": kw.get("del", 3), "fileCount": kw.get("files", 2)},
        "project": {"name": project, "fullPath": path},
        "state": kw.get("state", "opened"),
        "mergedAt": iso(kw["merged"]) if "merged" in kw else None, "closedAt": None,
        "labels": {"nodes": [{"title": t} for t in kw.get("labels", [])]},
        "description": kw.get("description", ""),
        "diffStats": kw.get("diffstats", []),
        "diffRefs": {"baseSha": sha(iid + 1), "headSha": sha(iid), "startSha": sha(iid + 1)},
        "squashOnMerge": True, "shouldRemoveSourceBranch": True, "forceRemoveSourceBranch": False,
        "commits": {"nodes": kw.get("commits", [])},
        "discussions": {"nodes": kw.get("discussions", [])},
        "_updated": NOW - ago, "_path": path,
    }
    node["jobs"] = pipe
    return node


def comment(user, text, ago, position=None):
    return {"body": text, "system": False, "createdAt": iso(ago), "author": person(user), "position": position}


def commit(n, title, user, ago):
    return {"shortId": sha(n)[:8], "title": title, "authorName": PEOPLE[user], "authoredDate": iso(ago),
            "webUrl": "%s/acme/team-a/api/-/commit/%s" % (GL_BASE, sha(n))}


STATS_DESC = (
    "Adds `GET /stats`, which answers with the request counts per route for the last hour.\n\n"
    "- counts are kept in memory and reset on restart\n- the route is documented in `docs/stats.md`\n\n"
    "Part of ABC-12."
)

GL_MRS = [
    mr("api", 263, "ABC-12: Add a stats endpoint", "alice", "feature/abc-12-stats", 25 * MIN,
       reviewers=["bob", "carol"], approved_by=["carol"], notes=3, threads=2, resolved=1,
       merge="DISCUSSIONS_NOT_RESOLVED", approved=True, labels=["backend"],
       checks=[("CI_MUST_PASS", "SUCCESS"), ("DISCUSSIONS_NOT_RESOLVED", "FAILED")],
       last=[comment("bob", "Could the counts be per route instead of per method?", 40 * MIN)],
       add=86, **{"del": 4}, files=3, description=STATS_DESC,
       diffstats=[{"path": "src/stats.rs", "additions": 61, "deletions": 0},
                  {"path": "src/routes.rs", "additions": 9, "deletions": 4},
                  {"path": "docs/stats.md", "additions": 16, "deletions": 0}],
       commits=[commit(2631, "Count requests per route", "alice", 3 * HOUR),
                commit(2632, "Add GET /stats and its docs", "alice", 2 * HOUR)],
       discussions=[
           {"id": "gid://gitlab/Discussion/d1", "resolved": False, "resolvable": True, "notes": {"nodes": [
               comment("bob", "Could the counts be per route instead of per method?", 40 * MIN,
                       {"filePath": "src/stats.rs", "newLine": 24, "oldLine": None}),
               comment("alice", "Good point, I will split them.", 30 * MIN,
                       {"filePath": "src/stats.rs", "newLine": 24, "oldLine": None})]}},
           {"id": "gid://gitlab/Discussion/d2", "resolved": True, "resolvable": True, "notes": {"nodes": [
               comment("carol", "Nit: name this `Counter`, not `Cnt`.", 90 * MIN,
                       {"filePath": "src/stats.rs", "newLine": 8, "oldLine": None}),
               comment("alice", "Done.", 80 * MIN, {"filePath": "src/stats.rs", "newLine": 8, "oldLine": None})]}},
           {"id": "gid://gitlab/Discussion/d3", "resolved": False, "resolvable": False, "notes": {"nodes": [
               comment("carol", "Looks good to me overall.", 20 * MIN)]}},
       ]),
    mr("api", 264, "Tidy the build", "bob", "abc-20-tidy", 2 * HOUR, pipeline="FAILED", reviewers=["alice"],
       merge="CI_MUST_PASS", checks=[("CI_MUST_PASS", "FAILED")], add=10, files=1, **{"del": 2}),
    mr("api", 265, "Draft: Cache user lookups", "carol", "carol/cache-users", 5 * HOUR, pipeline="RUNNING",
       draft=True, merge="DRAFT_STATUS", checks=[("CI_MUST_PASS", "CHECKING"), ("DRAFT_STATUS", "FAILED")],
       add=140, files=4, **{"del": 12}),
    mr("api", 262, "Fix the pagination off-by-one", "carol", "carol/page-fix", 2 * DAY, state="merged",
       merged=2 * DAY, approved=True, approved_by=["alice"], merge="NOT_OPEN", add=6, files=1, **{"del": 5}),
    mr("web", 118, "Update dependencies", "bob", "bob/deps", 3 * HOUR, reviewers=["alice"], approved=True,
       approved_by=["alice"], merge="MERGEABLE", add=48, files=2, **{"del": 40}),
    mr("docs", 41, "Document the stats endpoint", "alice", "abc-12-docs", 26 * MIN, reviewers=["bob"],
       merge="NOT_APPROVED", add=33, files=1, **{"del": 0}),
]

_DIFF = """@@ -1,4 +1,35 @@
+//! Request counts per route, kept in memory.
+use std::collections::BTreeMap;
+use std::sync::Mutex;
+
+#[derive(Default)]
+pub struct Counter {
+    routes: Mutex<BTreeMap<String, u64>>,
+}
+
+impl Counter {
+    /// Counts one request to `route`.
+    pub fn hit(&self, route: &str) {
+        *self.routes.lock().unwrap().entry(route.to_string()).or_default() += 1;
+    }
+
+    /// The counts so far, by route.
+    pub fn snapshot(&self) -> BTreeMap<String, u64> {
+        self.routes.lock().unwrap().clone()
+    }
+}
"""


def gl_diffs(path, iid):
    return [
        {"old_path": "src/stats.rs", "new_path": "src/stats.rs", "new_file": True, "renamed_file": False,
         "deleted_file": False, "too_large": False, "collapsed": False, "diff": _DIFF},
        {"old_path": "src/routes.rs", "new_path": "src/routes.rs", "new_file": False, "renamed_file": False,
         "deleted_file": False, "too_large": False, "collapsed": False,
         "diff": "@@ -10,6 +10,11 @@ pub fn routes() -> Router {\n     Router::new()\n         .route(\"/health\", get(health))\n-        .route(\"/\", get(index))\n+        .route(\"/\", get(index))\n+        .route(\"/stats\", get(stats))\n }\n"},
    ]


def gl_in_group(group, path):
    return path == group or path.startswith(group + "/")


def gl_mr_conn(scope):
    def conn(args):
        items = [m for m in GL_MRS if scope(m["_path"])]
        st = args.get("state")
        if st:
            items = [m for m in items if m["state"] == st]
        if args.get("updatedAfter"):
            items = [m for m in items if iso(NOW - m["_updated"]) >= args["updatedAfter"]]
        if args.get("iids"):
            items = [m for m in items if m["iid"] in args["iids"]]
        items.sort(key=lambda m: -m["_updated"])
        return connection(items, args)
    return conn


def gl_project_node(path):
    p = GL_PROJECTS.get(path)
    if not p:
        return None
    node = dict(p)
    node["__typename"] = "Project"
    node["nameWithNamespace"] = "Acme / Team A / " + p["name"]

    def pipelines(args):
        items = p["_pipelines"]
        if args.get("ref"):
            items = [x for x in items if x["ref"] == args["ref"]]
        return connection(items, args)

    def merge_request(args):
        for m in GL_MRS:
            if m["_path"] == path and m["iid"] == str(args.get("iid")):
                return m
        return None
    node["pipelines"] = pipelines
    node["mergeRequests"] = gl_mr_conn(lambda p_: p_ == path)
    node["mergeRequest"] = merge_request
    node["pipelines"] = pipelines
    return node


def gl_group_node(path):
    paths = [p for p in GL_PROJECTS if gl_in_group(path, p)]
    if not paths or path in GL_PROJECTS:
        return None
    return {
        "__typename": "Group", "fullName": path.replace("/", " / ").title(),
        "projects": lambda args: connection([gl_project_node(p) for p in paths], args),
        "mergeRequests": gl_mr_conn(lambda p: gl_in_group(path, p)),
    }


def gitlab_graphql(query, variables):
    root = {
        "currentUser": {"username": GL_ME, "name": PEOPLE[GL_ME]},
        "group": lambda a: gl_group_node(a.get("fullPath")),
        "project": lambda a: gl_project_node(a.get("fullPath")),
    }
    return graphql(query, variables, root)


def gitlab_rest(path, query):
    m = re.match(r"/api/v4/projects/([^/]+)/(.*)$", path)
    if not m:
        return None
    project, rest = urllib.parse.unquote(m.group(1)), m.group(2)
    d = re.match(r"merge_requests/(\d+)/diffs$", rest)
    if d:
        page = int(query.get("page", ["1"])[0])
        return 200, (gl_diffs(project, int(d.group(1))) if page == 1 else []), {}
    if re.match(r"merge_requests/(\d+)$", rest):
        return 200, {"iid": int(rest.split("/")[1]), "merge_status": "can_be_merged"}, {}
    if re.match(r"jobs/\d+/trace$", rest):
        text = ("\x1b[0KRunning with gitlab-runner 17.0\n$ cargo test\n   Compiling api v0.4.1\n"
                "running 14 tests\ntest result: ok. 14 passed; 0 failed\nJob succeeded\n")
        return 200, text, {"Content-Type": "text/plain"}
    return None


# ---------------------------------------------------------------- Jira
def adf(*paragraphs):
    return {"type": "doc", "version": 1, "content": [
        {"type": "paragraph", "content": [{"type": "text", "text": t}]} for t in paragraphs]}


def adf_list(intro, items):
    return {"type": "doc", "version": 1, "content": [
        {"type": "paragraph", "content": [{"type": "text", "text": intro}]},
        {"type": "bulletList", "content": [
            {"type": "listItem", "content": [{"type": "paragraph", "content": [{"type": "text", "text": i}]}]}
            for i in items]}]}


JIRA_USERS = {
    "alice": {"accountId": "acc-alice", "displayName": "Alice Example", "emailAddress": "alice@example.com"},
    "bob": {"accountId": "acc-bob", "displayName": "Bob Builder", "emailAddress": "bob@example.com"},
    "carol": {"accountId": "acc-carol", "displayName": "Carol Coder", "emailAddress": "carol@example.com"},
}
CAT = {"To Do": "new", "In Progress": "indeterminate", "In Review": "indeterminate", "Done": "done"}
STATUS_ID = {"To Do": "11", "In Progress": "21", "In Review": "31", "Done": "41"}
TYPES = {"Epic": ("10001", False), "Story": ("10002", False), "Task": ("10003", False),
         "Bug": ("10004", False), "Sub-task": ("10005", True)}
PRIO = ["Highest", "High", "Medium", "Low"]
JIRA_ISSUES = {}


def ticket(n, summary, kind, status, assignee, reporter, created_ago, updated_ago, **kw):
    kt, sub = TYPES[kind]
    f = {
        "summary": summary,
        "issuetype": {"id": kt, "name": kind, "subtask": sub},
        "status": {"name": status, "id": STATUS_ID[status], "statusCategory": {"key": CAT[status]}},
        "priority": {"name": kw.get("priority", "Medium")},
        "assignee": JIRA_USERS[assignee] if assignee else None,
        "reporter": JIRA_USERS[reporter],
        "created": jira_time(created_ago), "updated": jira_time(updated_ago),
        "labels": kw.get("labels", []), "components": [{"name": c} for c in kw.get("components", [])],
        "fixVersions": [{"name": v} for v in kw.get("versions", [])],
        "project": {"id": "10000", "key": "ABC", "name": "Acme Backend"},
        "description": kw.get("description") or adf(summary + "."),
        "issuelinks": kw.get("links", []),
        "customfield_10020": kw.get("sprint"),
        "attachment": [],
    }
    if kw.get("parent"):
        p = JIRA_ISSUES[kw["parent"]]["fields"]
        f["parent"] = {"key": kw["parent"], "fields": {"summary": p["summary"], "status": p["status"],
                                                      "issuetype": p["issuetype"]}}
    JIRA_ISSUES["ABC-%d" % n] = {"id": str(10000 + n), "key": "ABC-%d" % n, "fields": f,
                                 "_comments": kw.get("comments", []), "_remote": kw.get("remote", []),
                                 "_prs": kw.get("prs", [])}


SPRINT = [{"name": "Sprint 41", "state": "active"}]
ticket(1, "Usage dashboard", "Epic", "In Progress", "bob", "bob", 60 * DAY, 3 * DAY, priority="High")
ticket(9, "Move the request log to the new schema", "Task", "Done", "carol", "bob", 25 * DAY, 4 * DAY)
ticket(15, "Ship the stats endpoint to staging", "Task", "To Do", None, "bob", 12 * DAY, 2 * DAY, parent="ABC-1")
ticket(12, "Add a stats endpoint", "Story", "In Review", "alice", "bob", 24 * DAY, 25 * MIN, parent="ABC-1",
       priority="High", labels=["backend"], components=["api"], versions=["1.4"], sprint=SPRINT,
       description=adf_list("The dashboard needs request counts per route. Add GET /stats to the API:",
                            ["answers with JSON, one entry per route", "counts reset when the service restarts",
                             "documented in docs/stats.md"]),
       links=[{"type": {"name": "Blocks", "inward": "is blocked by", "outward": "blocks"},
               "outwardIssue": {"key": "ABC-15", "fields": {"summary": "Ship the stats endpoint to staging",
                                "status": {"name": "To Do", "statusCategory": {"key": "new"}}}}},
              {"type": {"name": "Blocks", "inward": "is blocked by", "outward": "blocks"},
               "inwardIssue": {"key": "ABC-9", "fields": {"summary": "Move the request log to the new schema",
                               "status": {"name": "Done", "statusCategory": {"key": "done"}}}}}],
       comments=[("bob", 6 * HOUR, "Can you also expose the p95 latency? The dashboard has a tile for it."),
                 ("alice", 5 * HOUR, "Not in this ticket; I will open a follow-up once the counts are in."),
                 ("carol", 90 * MIN, "Reviewed the merge request, a few nits only.")],
       remote=[("Design note: usage dashboard", "https://wiki.example/acme/usage-dashboard")],
       prs=[{"id": "acme/team-a/api!263", "name": "ABC-12: Add a stats endpoint", "status": "OPEN",
             "repositoryName": "api", "source": {"branch": "feature/abc-12-stats"},
             "url": GL_BASE + "/acme/team-a/api/-/merge_requests/263"}])
ticket(20, "Tidy the build", "Task", "In Progress", "bob", "alice", 5 * DAY, 2 * HOUR, parent="ABC-1", sprint=SPRINT)
ticket(21, "Write the route counter", "Sub-task", "Done", "alice", "alice", 20 * DAY, 3 * DAY, parent="ABC-12")
ticket(22, "Document the stats endpoint", "Sub-task", "In Progress", "alice", "alice", 20 * DAY, 30 * MIN,
       parent="ABC-12")
ticket(23, "Stats endpoint returns 500 for an unknown route", "Bug", "To Do", "carol", "bob", 2 * DAY, DAY,
       priority="Highest", labels=["backend"], components=["api"], sprint=SPRINT)
ticket(24, "Cache user lookups", "Story", "In Progress", "carol", "alice", 9 * DAY, 5 * HOUR, parent="ABC-1",
       sprint=SPRINT)


def jira_issue_json(key):
    i = JIRA_ISSUES[key]
    return {"id": i["id"], "key": key, "fields": i["fields"],
            "names": {"customfield_10020": "Sprint"}}


def jql_filter(jql):
    jql = re.sub(r"\s+ORDER\s+BY\s+.*$", "", jql, flags=re.I)
    items = list(JIRA_ISSUES.values())
    m = re.search(r"parent\s*=\s*\"?([A-Z]+-\d+)", jql)
    if m:
        items = [i for i in items if i["fields"].get("parent", {}).get("key") == m.group(1)]
    m = re.search(r"\bkey\s+in\s*\(([^)]*)\)", jql, re.I)
    if m:
        keys = [k.strip().strip('"') for k in m.group(1).split(",")]
        items = [i for i in items if i["key"] in keys]
    m = re.search(r"\bproject\s*=\s*\"?([A-Za-z]+)", jql)
    if m:
        items = [i for i in items if i["fields"]["project"]["key"] == m.group(1)]
    if re.search(r"assignee\s*=\s*currentUser\(\)", jql, re.I):
        items = [i for i in items if (i["fields"]["assignee"] or {}).get("accountId") == "acc-alice"]
    if re.search(r"statusCategory\s*!=\s*Done|resolution\s*=\s*Unresolved", jql, re.I):
        items = [i for i in items if i["fields"]["status"]["statusCategory"]["key"] != "done"]
    items.sort(key=lambda i: i["fields"]["updated"], reverse=True)
    return items


def jira_rest(path, q):
    if path == "/rest/api/3/myself":
        return 200, JIRA_USERS["alice"]
    if path == "/rest/api/3/search/jql":
        jql = q.get("jql", [""])[0]
        issues = [jira_issue_json(i["key"]) for i in jql_filter(jql)]
        if "fields" in q and q["fields"][0] == "updated":
            for x in issues:
                x["fields"] = {"updated": x["fields"]["updated"]}
        return 200, {"issues": issues, "isLast": True}
    if path == "/rest/api/3/search/approximate-count":
        return 200, {"count": 12}
    if path == "/rest/api/3/jql/parse":
        return 200, {"queries": [{"errors": []}]}
    if path == "/rest/api/3/field":
        return 200, [{"id": "customfield_10020", "name": "Sprint"}, {"id": "summary", "name": "Summary"}]
    if path == "/rest/api/3/project/recent":
        return 200, [{"key": "ABC", "name": "Acme Backend"}]
    if path == "/rest/api/3/project/ABC":
        return 200, {"id": "10000", "key": "ABC", "name": "Acme Backend"}
    m = re.match(r"/rest/api/3/issue/createmeta/ABC/issuetypes$", path)
    if m:
        return 200, {"issueTypes": [{"id": i, "name": n, "subtask": s} for n, (i, s) in TYPES.items()]}
    m = re.match(r"/rest/api/3/issue/(ABC-\d+)(?:/(\w+))?$", path)
    if m and m.group(1) in JIRA_ISSUES:
        key, what = m.group(1), m.group(2)
        i = JIRA_ISSUES[key]
        if what is None:
            out = jira_issue_json(key)
            if q.get("fields", [""])[0] == "updated":
                out["fields"] = {"updated": i["fields"]["updated"]}
            return 200, out
        if what == "comment":
            cs = [{"id": str(100 + n), "author": JIRA_USERS[u], "created": jira_time(ago), "body": adf(t)}
                  for n, (u, ago, t) in enumerate(i["_comments"])]
            return 200, {"comments": list(reversed(cs)), "total": len(cs), "startAt": 0, "maxResults": 50}
        if what == "remotelink":
            return 200, [{"id": 1, "relationship": "links to", "object": {"title": t, "url": u}}
                         for t, u in i["_remote"]]
        if what == "transitions":
            return 200, {"transitions": [{"id": "31", "name": "In Review", "to": {"name": "In Review",
                         "statusCategory": {"key": "indeterminate"}}}, {"id": "41", "name": "Done",
                         "to": {"name": "Done", "statusCategory": {"key": "done"}}}]}
    m = re.match(r"/rest/dev-status/latest/issue/summary$", path)
    if m:
        n = len([x for x in JIRA_ISSUES.values() if x["id"] == q.get("issueId", [""])[0] and x["_prs"]])
        return 200, {"summary": {"pullrequest": {"overall": {"count": n},
                                                 "byInstanceType": {"GitLab": {"count": n}} if n else {}}}}
    if path == "/rest/dev-status/latest/issue/detail":
        prs = [p for x in JIRA_ISSUES.values() if x["id"] == q.get("issueId", [""])[0] for p in x["_prs"]]
        return 200, {"detail": [{"pullRequests": prs}]}
    return None


# ---------------------------------------------------------------- GitHub
GH_REPOS = {
    "acme-labs/widgets": ("Small widgets for terminal dashboards", "main"),
    "acme-labs/toolkit": ("Shared build helpers", "main"),
}
GH_USERS = {"alice": 1001, "bob": 1002, "carol": 1003}


def gh_pull(repo, n, title, user, branch, ago, draft=False, body=""):
    return {"number": n, "repo": repo, "title": title, "user": user, "branch": branch, "ago": ago, "draft": draft,
            "body": body, "sha": sha(7000 + n)}


GH_PULLS = [
    gh_pull("acme-labs/widgets", 58, "Add a sparkline widget", "alice", "alice/sparkline", 50 * MIN,
            body="Adds `Sparkline`, a one-row chart for a series of numbers.\n\nCloses #41."),
    gh_pull("acme-labs/widgets", 57, "Fix the gauge when the maximum is zero", "bob", "bob/gauge-zero", 6 * HOUR,
            body="A gauge with a maximum of zero divided by zero. It now draws empty."),
    gh_pull("acme-labs/widgets", 55, "Draft: table column sizing", "carol", "carol/columns", 2 * DAY, True),
    gh_pull("acme-labs/toolkit", 12, "Cache the downloaded toolchain", "bob", "bob/cache", 4 * HOUR,
            body="Keeps the toolchain between runs."),
]


def gh_repo_json(name):
    desc, branch = GH_REPOS[name]
    owner, short = name.split("/")
    return {"id": 5000 + sorted(GH_REPOS).index(name), "full_name": name, "name": short,
            "owner": {"login": owner}, "html_url": "https://github.com/" + name,
            "description": desc, "default_branch": branch, "private": False, "archived": False}


def gh_pull_json(p, full=False):
    r = gh_repo_json(p["repo"])
    out = {"number": p["number"], "html_url": "https://github.com/%s/pull/%d" % (p["repo"], p["number"]),
           "title": p["title"], "body": p["body"] or None, "user": {"login": p["user"]}, "state": "open",
           "draft": p["draft"], "updated_at": iso(p["ago"]),
           "head": {"sha": p["sha"], "ref": p["branch"], "repo": r},
           "base": {"sha": sha(6000 + p["number"]), "ref": "main", "repo": r}}
    if full:
        out["merged"] = False
    return out


def gh_graphql(query, variables):
    def repo(args):
        name = "%s/%s" % (args.get("owner"), args.get("name"))
        if name not in GH_REPOS:
            return None
        desc, branch = GH_REPOS[name]
        pulls = sorted([p for p in GH_PULLS if p["repo"] == name], key=lambda p: p["ago"])
        nodes = [{"number": p["number"], "title": p["title"], "body": p["body"], "isDraft": p["draft"],
                  "headRefOid": p["sha"], "headRefName": p["branch"], "baseRefName": "main",
                  "updatedAt": iso(p["ago"]), "author": {"login": p["user"]},
                  "headRepository": {"nameWithOwner": name}} for p in pulls]
        total = len(nodes)
        return {"nameWithOwner": name, "description": desc, "isPrivate": False, "isArchived": False,
                "defaultBranchRef": {"name": branch},
                "pullRequests": lambda a: {"totalCount": total, "nodes": nodes[:a.get("first", total)]}}
    return graphql(query, variables, {"repository": repo})


def github_rest(path, q):
    if path == "/rate_limit":
        return 200, {"resources": {"core": {"limit": 5000, "remaining": 4990, "reset": NOW + 3000}}}
    if path == "/user":
        return 200, {"id": GH_USERS["alice"], "login": "alice"}
    if path == "/user/repos":
        return 200, [gh_repo_json(n) for n in sorted(GH_REPOS)]
    m = re.match(r"/repos/([^/]+/[^/]+)(?:/(.*))?$", path)
    if not m or m.group(1) not in GH_REPOS:
        return None
    name, rest = m.group(1), m.group(2)
    if rest is None:
        return 200, gh_repo_json(name)
    if rest == "pulls":
        page = int(q.get("page", ["1"])[0])
        pulls = sorted([p for p in GH_PULLS if p["repo"] == name], key=lambda p: p["ago"])
        return 200, [gh_pull_json(p) for p in pulls] if page == 1 else []
    m = re.match(r"pulls/(\d+)$", rest)
    if m:
        for p in GH_PULLS:
            if p["repo"] == name and p["number"] == int(m.group(1)):
                return 200, gh_pull_json(p, True)
        return 404, {"message": "Not Found"}
    m = re.match(r"commits/([0-9a-f]{40})/(check-runs|statuses)$", rest)
    if m:
        page = int(q.get("page", ["1"])[0])
        s = m.group(1)
        if m.group(2) == "check-runs":
            runs = [{"id": 9000 + k, "name": n, "status": "completed", "conclusion": c, "head_sha": s}
                    for k, (n, c) in enumerate([("build", "success"), ("test", "success"), ("lint", "success")])]
            return 200, {"total_count": len(runs), "check_runs": runs if page == 1 else []}
        return 200, ([{"id": 8000, "url": "https://api.github.com/repos/%s/statuses/%s" % (name, s),
                       "context": "ci/release-notes", "state": "success",
                       "description": "Release notes found"}] if page == 1 else [])
    return None


# ---------------------------------------------------------------- Slack
SL_TEAM_ID, SL_ME = "T000ACME1", "U0000ALICE"
SL_USERS = {
    "U0000ALICE": ("alice", "Alice Example"),
    "U00000BOB1": ("bob", "Bob Builder"),
    "U0000CAROL": ("carol", "Carol Coder"),
}
SL_GROUP = {"id": "S0REVIEW01", "handle": "review-squad", "name": "Review squad",
            "users": ["U0000ALICE", "U00000BOB1"]}
MR_263 = GL_BASE + "/acme/team-a/api/-/merge_requests/263"
SL_CHANNELS = {
    "C0000TEAM1": ("acme-team", "where the team talks"),
    "C00000SVC1": ("acme-svc", "the build service and its upgrades"),
    "C00000TRY1": ("acme-try", "for trying things out"),
}


def ts(ago, n=0):
    return "%d.%06d" % (NOW - ago, 100 + n)


def slmsg(ago, user, text, n=0, **kw):
    m = {"type": "message", "ts": ts(ago, n), "user": user, "text": text}
    m.update(kw)
    return m


SL_ASK_TS = ts(2 * DAY + HOUR)
SL_Q_TS = ts(DAY + 10 * MIN, 1)
SL_THREADS = {
    SL_ASK_TS: [slmsg(2 * DAY + 30 * MIN, "U0000ALICE", "looking :eyes:", 3, thread_ts=SL_ASK_TS),
                slmsg(2 * DAY, "U0000CAROL", "approved, nice one", 4, thread_ts=SL_ASK_TS)],
    SL_Q_TS: [slmsg(DAY, "U0000ALICE", "<@U00000BOB1> can you look? see " + MR_263, 5, thread_ts=SL_Q_TS)],
}
SL_MESSAGES = {
    "C0000TEAM1": [
        slmsg(2 * DAY + HOUR, "U00000BOB1",
              "<!subteam^S0REVIEW01> :gitlab: <%s|mr> Add a stats endpoint for the dashboard" % MR_263, 0,
              thread_ts=SL_ASK_TS, reply_count=2, reply_users=["U0000ALICE", "U0000CAROL"],
              reply_users_count=2, latest_reply=ts(2 * DAY, 4),
              reactions=[{"name": "eyes", "count": 1, "users": ["U0000ALICE"]}]),
        slmsg(DAY + 10 * MIN, "U0000CAROL",
              "Anyone seen *ABC-12* failing on main? <%s/acme/team-a/api/-/pipelines/99|pipeline> and `make check`" % GL_BASE,
              1, thread_ts=SL_Q_TS, reply_count=1, reply_users=["U0000ALICE"], reply_users_count=1,
              latest_reply=ts(DAY, 5)),
        slmsg(DAY + 5 * MIN, "U00000BOB1", "<@U0000ALICE> could you take this one?", 2),
        slmsg(30 * MIN, "U00000BOB1", "Lunch at 12:30? :pizza:", 6),
    ],
    "C00000SVC1": [slmsg(5 * HOUR, "U0000CAROL", "The build service is being upgraded at noon", 7)],
    "C00000TRY1": [],
}
SL_DM = {"id": "D0000CAROL", "msg": slmsg(2 * HOUR, "U0000CAROL", "hi <@U0000ALICE>, quick question about the stats", 8)}


def sl_channel_json(cid):
    name, purpose = SL_CHANNELS[cid]
    return {"id": cid, "name": name, "is_channel": True, "is_private": False, "is_member": True,
            "is_archived": False, "is_im": False, "topic": {"value": ""}, "purpose": {"value": purpose}}


def sl_matches(query):
    out = []

    def add(cid, cname, m, is_im=False):
        out.append({"channel": {"id": cid, "name": cname, "is_im": is_im, "is_mpim": False},
                    "permalink": "https://acme.slack.com/archives/%s/p%s" % (cid, m["ts"].replace(".", "")),
                    **m})
    texts = []
    for cid, msgs in SL_MESSAGES.items():
        for m in msgs:
            texts.append((cid, SL_CHANNELS[cid][0], m, False))
    texts.append((SL_DM["id"], "U0000CAROL", SL_DM["msg"], True))
    for cid, cname, m, im in texts:
        if "to:me" in query:
            hit = im
        else:
            hit = False
            for tok in re.findall(r"<[@!][^>\s]+>", query):
                hit = hit or tok in m["text"]
        if hit and m["user"] != SL_ME:
            add(cid, cname, m, im)
    out.sort(key=lambda x: x["ts"], reverse=True)
    return out


def slack_call(method, params):
    ok = lambda **kw: dict(ok=True, **kw)
    if method == "auth.test":
        return ok(url="https://acme.slack.com/", team="Acme", user="alice", team_id=SL_TEAM_ID, user_id=SL_ME)
    if method == "users.info":
        u = SL_USERS.get(params.get("user"))
        if not u:
            return {"ok": False, "error": "user_not_found"}
        return ok(user={"id": params["user"], "name": u[0], "tz_offset": 0,
                        "profile": {"real_name": u[1], "display_name": u[0]}})
    if method == "usergroups.list":
        return ok(usergroups=[SL_GROUP])
    if method in ("users.conversations", "conversations.list"):
        return ok(channels=[sl_channel_json(c) for c in SL_CHANNELS], response_metadata={"next_cursor": ""})
    if method == "conversations.info":
        if params.get("channel") not in SL_CHANNELS:
            return {"ok": False, "error": "channel_not_found"}
        return ok(channel=sl_channel_json(params["channel"]))
    if method == "conversations.history":
        msgs = SL_MESSAGES.get(params.get("channel"))
        if msgs is None:
            return {"ok": False, "error": "channel_not_found"}
        oldest = float(params.get("oldest") or 0)
        msgs = sorted([m for m in msgs if float(m["ts"]) > oldest], key=lambda m: m["ts"], reverse=True)
        return ok(messages=msgs[:int(params.get("limit", 100))], has_more=False,
                  response_metadata={"next_cursor": ""})
    if method == "conversations.replies":
        parent = [m for m in SL_MESSAGES.get(params.get("channel"), []) if m["ts"] == params.get("ts")]
        return ok(messages=parent + SL_THREADS.get(params.get("ts"), []), has_more=False,
                  response_metadata={"next_cursor": ""})
    if method == "search.messages":
        found = sl_matches(params.get("query", ""))
        return ok(messages={"matches": found, "total": len(found),
                            "paging": {"count": len(found), "total": len(found), "page": 1, "pages": 1}})
    if method == "chat.postMessage":
        return ok(channel=params.get("channel", ""), ts=ts(0, 99))
    if method == "reactions.add":
        return ok()
    return None


# ---------------------------------------------------------------- HTTP
SLACK_SCOPES = ("channels:history,channels:read,groups:history,groups:read,search:read,"
                "users:read,usergroups:read,chat:write,reactions:write")


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        pass

    def reply(self, status, body, extra=None):
        if isinstance(body, str):
            data, ctype = body.encode(), "text/plain; charset=utf-8"
        else:
            data, ctype = json.dumps(body).encode(), "application/json; charset=utf-8"
        self.send_response(status)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(data)))
        for k, v in (extra or {}).items():
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(data)

    def handle_any(self):
        url = urllib.parse.urlsplit(self.path)
        path, q = url.path, urllib.parse.parse_qs(url.query)
        length = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(length) if length else b""
        body = None
        if raw:
            try:
                body = json.loads(raw)
            except ValueError:
                body = {k: v[0] for k, v in urllib.parse.parse_qs(raw.decode()).items()}
        try:
            if path == "/api/graphql" and body:
                return self.reply(200, gitlab_graphql(body.get("query", ""), body.get("variables")))
            if path.startswith("/api/v4/"):
                got = gitlab_rest(path, q)
                if got:
                    return self.reply(got[0], got[1], got[2])
            if path.startswith("/rest/"):
                got = jira_rest(path, q)
                if got:
                    return self.reply(got[0], got[1])
            if path == "/graphql" and body:
                return self.reply(200, gh_graphql(body.get("query", ""), body.get("variables")))
            if path.startswith("/api/") and not path.startswith("/api/v4/"):
                method = path[len("/api/"):]
                params = {k: v[0] for k, v in q.items()}
                if isinstance(body, dict):
                    params.update({k: v for k, v in body.items() if isinstance(v, (str, int))})
                got = slack_call(method, params)
                if got is not None:
                    return self.reply(200, got, {"x-oauth-scopes": SLACK_SCOPES})
            if self.command == "GET":
                got = github_rest(path, q)
                if got:
                    return self.reply(got[0], got[1], {"x-ratelimit-limit": "5000", "x-ratelimit-remaining": "4990",
                                                       "x-ratelimit-reset": str(NOW + 3000),
                                                       "x-ratelimit-resource": "core"})
        except Exception as e:  # a gap in the sample data must show, not hang the client
            log("ERROR %s %s: %r" % (self.command, self.path, e))
            return self.reply(500, {"message": "fake-services error: %r" % e})
        log("UNKNOWN %s %s %s" % (self.command, self.path, (raw[:200].decode("utf-8", "replace") if raw else "")))
        self.reply(404, {"message": "Not Found", "ok": False, "error": "unknown_method"})

    do_GET = do_POST = do_PUT = handle_any


def main():
    port = 0
    if "--port" in sys.argv:
        port = int(sys.argv[sys.argv.index("--port") + 1])
    srv = ThreadingHTTPServer(("127.0.0.1", port), Handler)
    srv.daemon_threads = True
    print("http://127.0.0.1:%d" % srv.server_address[1], flush=True)
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
