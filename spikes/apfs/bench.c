/*
 * APFS spike benchmark: measures the three primitives an APFS snapshot
 * backend would be built on, against a repo-sized tree (100 dirs x 100
 * files = 10k files by default).
 *
 *   1. clonefile(2) of the whole directory tree (the "snapshot" at run time)
 *   2. tree-comparison diff: parallel walk of snapshot vs live tree,
 *      stat-based (size + mtimespec) modification detection
 *   3. renamex_np(RENAME_SWAP): atomic directory swap (the "undo")
 *
 * Usage: ./bench <work-dir> [n_dirs] [files_per_dir]
 * The work dir must be on APFS. Everything is created under it.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/attr.h>
#include <sys/clonefile.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <dirent.h>

static double now_ms(void) {
    struct timeval tv;
    gettimeofday(&tv, NULL);
    return tv.tv_sec * 1000.0 + tv.tv_usec / 1000.0;
}

static void die(const char *what) {
    fprintf(stderr, "FATAL: %s: %s\n", what, strerror(errno));
    exit(1);
}

static void write_file(const char *path, const char *content) {
    FILE *f = fopen(path, "w");
    if (!f) die(path);
    fputs(content, f);
    fclose(f);
}

static void build_tree(const char *root, int n_dirs, int n_files) {
    char path[1024];
    if (mkdir(root, 0755) != 0) die(root);
    for (int d = 0; d < n_dirs; d++) {
        snprintf(path, sizeof path, "%s/dir%03d", root, d);
        if (mkdir(path, 0755) != 0) die(path);
        for (int f = 0; f < n_files; f++) {
            snprintf(path, sizeof path, "%s/dir%03d/file%03d.txt", root, d, f);
            write_file(path, "some file content that is not empty\n");
        }
    }
}

/* Parallel walk: everything in `snap` is compared against `live`.
 * Returns entries visited; counts adds/mods/dels like a real diff would. */
struct diffstat { long visited, added, modified, deleted; };

static void diff_walk(const char *snap, const char *live, struct diffstat *st) {
    DIR *dir = opendir(snap);
    if (!dir) die(snap);
    struct dirent *e;
    char spath[1024], lpath[1024];
    while ((e = readdir(dir))) {
        if (strcmp(e->d_name, ".") == 0 || strcmp(e->d_name, "..") == 0) continue;
        snprintf(spath, sizeof spath, "%s/%s", snap, e->d_name);
        snprintf(lpath, sizeof lpath, "%s/%s", live, e->d_name);
        st->visited++;
        struct stat ss, ls;
        if (lstat(spath, &ss) != 0) die(spath);
        if (lstat(lpath, &ls) != 0) {
            st->deleted++; /* prune: do not descend into deleted dirs */
            continue;
        }
        if (S_ISDIR(ss.st_mode)) {
            diff_walk(spath, lpath, st);
        } else if (ss.st_size != ls.st_size ||
                   ss.st_mtimespec.tv_sec != ls.st_mtimespec.tv_sec ||
                   ss.st_mtimespec.tv_nsec != ls.st_mtimespec.tv_nsec) {
            st->modified++;
        }
    }
    closedir(dir);
}

/* Reverse pass for additions: entries in live missing from snap. */
static void adds_walk(const char *live, const char *snap, struct diffstat *st) {
    DIR *dir = opendir(live);
    if (!dir) die(live);
    struct dirent *e;
    char spath[1024], lpath[1024];
    while ((e = readdir(dir))) {
        if (strcmp(e->d_name, ".") == 0 || strcmp(e->d_name, "..") == 0) continue;
        snprintf(lpath, sizeof lpath, "%s/%s", live, e->d_name);
        snprintf(spath, sizeof spath, "%s/%s", snap, e->d_name);
        st->visited++;
        struct stat ss, ls;
        if (lstat(lpath, &ls) != 0) die(lpath);
        if (lstat(spath, &ss) != 0) {
            st->added++;
            continue; /* prune: whole subtree is new */
        }
        if (S_ISDIR(ls.st_mode)) adds_walk(lpath, spath, st);
    }
    closedir(dir);
}

int main(int argc, char **argv) {
    if (argc < 2) {
        fprintf(stderr, "usage: %s <work-dir> [n_dirs] [files_per_dir]\n", argv[0]);
        return 2;
    }
    const char *base = argv[1];
    int n_dirs = argc > 2 ? atoi(argv[2]) : 100;
    int n_files = argc > 3 ? atoi(argv[3]) : 100;

    char live[1024], snap[1024], cmd[2048];
    snprintf(live, sizeof live, "%s/live", base);
    snprintf(snap, sizeof snap, "%s/snap", base);
    snprintf(cmd, sizeof cmd, "rm -rf '%s' '%s'", live, snap);
    system(cmd);

    printf("tree: %d dirs x %d files = %d files\n", n_dirs, n_files, n_dirs * n_files);
    double t0 = now_ms();
    build_tree(live, n_dirs, n_files);
    printf("build_tree:            %8.2f ms\n", now_ms() - t0);

    /* 1. snapshot via clonefile(2) — one syscall for the whole tree */
    t0 = now_ms();
    if (clonefile(live, snap, 0) != 0) die("clonefile");
    double t_clone = now_ms() - t0;
    printf("clonefile (tree):      %8.2f ms\n", t_clone);

    /* simulate a command's damage: delete one subdir, modify 10, add 5 */
    char path[1024];
    snprintf(cmd, sizeof cmd, "rm -rf '%s/dir%03d'", live, 0);
    system(cmd);
    int n_mod = n_dirs > 10 ? 10 : n_dirs - 1;
    for (int i = 1; i <= n_mod; i++) {
        snprintf(path, sizeof path, "%s/dir%03d/file000.txt", live, i);
        write_file(path, "MODIFIED\n");
    }
    for (int i = 0; i < 5; i++) {
        snprintf(path, sizeof path, "%s/newfile%d.txt", live, i);
        write_file(path, "NEW\n");
    }

    /* 2. tree-comparison diff */
    struct diffstat st = {0, 0, 0, 0};
    t0 = now_ms();
    diff_walk(snap, live, &st);
    adds_walk(live, snap, &st);
    double t_diff = now_ms() - t0;
    printf("diff (2-way walk):     %8.2f ms  (%ld stats; A=%ld M=%ld D=%ld)\n",
           t_diff, st.visited, st.added, st.modified, st.deleted);

    /* 3. undo via atomic swap */
    t0 = now_ms();
    if (renamex_np(snap, live, RENAME_SWAP) != 0) die("renamex_np(RENAME_SWAP)");
    double t_swap = now_ms() - t0;
    printf("renamex_np SWAP:       %8.2f ms  (atomic undo)\n", t_swap);

    /* verify: live is pristine again (dir000 back, no newfiles) */
    struct stat sb;
    snprintf(path, sizeof path, "%s/dir%03d/file000.txt", live, 0);
    int restored = (lstat(path, &sb) == 0);
    snprintf(path, sizeof path, "%s/newfile0.txt", live);
    int clean = (lstat(path, &sb) != 0);
    snprintf(path, sizeof path, "%s/dir001/file000.txt", live);
    FILE *f = fopen(path, "r");
    char buf[64] = {0};
    if (f) { fgets(buf, sizeof buf, f); fclose(f); }
    int unmodified = (strncmp(buf, "some file", 9) == 0);
    printf("verify after swap:     restored=%s clean=%s unmodified=%s\n",
           restored ? "yes" : "NO", clean ? "yes" : "NO", unmodified ? "yes" : "NO");

    printf("\nsummary_ms clone=%.2f diff=%.2f swap=%.2f\n", t_clone, t_diff, t_swap);
    return (restored && clean && unmodified) ? 0 : 1;
}
