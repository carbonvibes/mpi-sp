#ifndef __VMLINUX_H__
#define __VMLINUX_H__

typedef signed int __s32;
typedef unsigned int __u32;
typedef unsigned int uint;
typedef __u32 u32;
typedef __u32 int32;
typedef __u32 __be32;
typedef u32 uint32_t;
typedef u32 u_int32_t;
typedef __u32 __wsum;

typedef short unsigned int __be16;

typedef long long unsigned int __u64;
typedef __u64 u64;
typedef __u64 __le64;
typedef __u64 __be64;
typedef u64 uint64_t;
typedef u64 u_int64_t;

struct pt_regs {
  long unsigned int r15;
  long unsigned int r14;
  long unsigned int r13;
  long unsigned int r12;
  long unsigned int bp;
  long unsigned int bx;
  long unsigned int r11;
  long unsigned int r10;
  long unsigned int r9;
  long unsigned int r8;
  long unsigned int ax;
  long unsigned int cx;
  long unsigned int dx;
  long unsigned int si;
  long unsigned int di;
  long unsigned int orig_ax;
  long unsigned int ip;
  long unsigned int cs;
  long unsigned int flags;
  long unsigned int sp;
  long unsigned int ss;
};

struct path {
  struct vfsmount *mnt;
  struct dentry *dentry;
};

typedef unsigned short umode_t;

struct seqcount {
  unsigned int sequence;
};

typedef struct seqcount seqcount_t;

typedef struct {
  seqcount_t seqcount;
} seqcount_latch_t;

struct seqcount_spinlock {
  seqcount_t seqcount;
};

typedef struct seqcount_spinlock seqcount_spinlock_t;

struct hlist_bl_node {
  struct hlist_bl_node *next;
  struct hlist_bl_node **pprev;
};

struct qstr {
  union {
    struct {
      u32 hash;
      u32 len;
    };
    u64 hash_len;
  };
  const unsigned char *name;
};

typedef struct {
  int counter;
} atomic_t;

union shortname_store {
  unsigned char string[40];
  long unsigned int words[5];
};

typedef unsigned char __u8;
typedef __u8 u8;

typedef short unsigned int __u16;
typedef __u16 u16;

struct qspinlock {
  union {
    atomic_t val;
    struct {
      u8 locked;
      u8 pending;
    };
    struct {
      u16 locked_pending;
      u16 tail;
    };
  };
};

typedef struct qspinlock arch_spinlock_t;

struct raw_spinlock {
  arch_spinlock_t raw_lock;
};

struct spinlock {
  union {
    struct raw_spinlock rlock;
  };
};

typedef struct spinlock spinlock_t;

struct lockref {
  union {
    __u64 lock_count;
    struct {
      spinlock_t lock;
      int count;
    };
  };
};

struct dentry_operations;

struct list_head {
  struct list_head *next;
  struct list_head *prev;
};

struct wait_queue_head {
  spinlock_t lock;
  struct list_head head;
};

typedef struct wait_queue_head wait_queue_head_t;

struct hlist_head {
  struct hlist_node *first;
};

struct hlist_node {
  struct hlist_node *next;
  struct hlist_node **pprev;
};

struct callback_head {
  struct callback_head *next;
  void (*func)(struct callback_head *);
};

struct dentry {
  unsigned int d_flags;
  seqcount_spinlock_t d_seq;
  struct hlist_bl_node d_hash;
  struct dentry *d_parent;
  struct qstr d_name;
  struct inode *d_inode;
  union shortname_store d_shortname;
  const struct dentry_operations *d_op;
  struct super_block *d_sb;
  long unsigned int d_time;
  void *d_fsdata;
  struct lockref d_lockref;
  union {
    struct list_head d_lru;
    wait_queue_head_t *d_wait;
  };
  struct hlist_node d_sib;
  struct hlist_head d_children;
  union {
    struct hlist_node d_alias;
    struct hlist_bl_node d_in_lookup_hash;
    struct callback_head d_rcu;
  } d_u;
};

typedef unsigned int __kernel_gid32_t;
typedef __kernel_gid32_t gid_t;

typedef unsigned int __kernel_uid32_t;
typedef __kernel_uid32_t uid_t;

typedef u32 __kernel_dev_t;
typedef __kernel_dev_t dev_t;

typedef struct {
  uid_t val;
} kuid_t;

typedef struct {
  gid_t val;
} kgid_t;

typedef long long int __kernel_loff_t;
typedef __kernel_loff_t loff_t;

typedef long long int __s64;
typedef __s64 time64_t;

enum rw_hint {
  WRITE_LIFE_NOT_SET = 0,
  WRITE_LIFE_NONE = 1,
  WRITE_LIFE_SHORT = 2,
  WRITE_LIFE_MEDIUM = 3,
  WRITE_LIFE_LONG = 4,
  WRITE_LIFE_EXTREME = 5,
} __attribute__((mode(byte)));

typedef u64 blkcnt_t;

typedef struct raw_spinlock raw_spinlock_t;

struct optimistic_spin_queue {
  atomic_t tail;
};

typedef __s64 s64;

typedef struct {
  s64 counter;
} atomic64_t;

typedef atomic64_t atomic_long_t;

struct rw_semaphore {
  atomic_long_t count;
  atomic_long_t owner;
  struct optimistic_spin_queue osq;
  raw_spinlock_t wait_lock;
  struct list_head wait_list;
};

typedef unsigned int gfp_t;

struct xarray {
  spinlock_t xa_lock;
  gfp_t xa_flags;
  void *xa_head;
};

struct rb_node {
  long unsigned int __rb_parent_color;
  struct rb_node *rb_right;
  struct rb_node *rb_left;
};

struct rb_root {
  struct rb_node *rb_node;
};

struct rb_root_cached {
  struct rb_root rb_root;
  struct rb_node *rb_leftmost;
};

typedef u32 errseq_t;

struct address_space {
  struct inode *host;
  struct xarray i_pages;
  struct rw_semaphore invalidate_lock;
  gfp_t gfp_mask;
  atomic_t i_mmap_writable;
  struct rb_root_cached i_mmap;
  long unsigned int nrpages;
  long unsigned int writeback_index;
  const struct address_space_operations *a_ops;
  long unsigned int flags;
  errseq_t wb_err;
  spinlock_t i_private_lock;
  struct list_head i_private_list;
  struct rw_semaphore i_mmap_rwsem;
  void *i_private_data;
};

struct inode {
  umode_t i_mode;
  short unsigned int i_opflags;
  kuid_t i_uid;
  kgid_t i_gid;
  unsigned int i_flags;
  struct posix_acl *i_acl;
  struct posix_acl *i_default_acl;
  const struct inode_operations *i_op;
  struct super_block *i_sb;
  struct address_space *i_mapping;
  void *i_security;
  long unsigned int i_ino;
  union {
    const unsigned int i_nlink;
    unsigned int __i_nlink;
  };
  dev_t i_rdev;
  loff_t i_size;
  time64_t i_atime_sec;
  time64_t i_mtime_sec;
  time64_t i_ctime_sec;
  u32 i_atime_nsec;
  u32 i_mtime_nsec;
  u32 i_ctime_nsec;
  u32 i_generation;
  spinlock_t i_lock;
  short unsigned int i_bytes;
  u8 i_blkbits;
  enum rw_hint i_write_hint;
  blkcnt_t i_blocks;
  u32 i_state;
  struct rw_semaphore i_rwsem;
  long unsigned int dirtied_when;
  long unsigned int dirtied_time_when;
  struct hlist_node i_hash;
  struct list_head i_io_list;
  struct bdi_writeback *i_wb;
  int i_wb_frn_winner;
  u16 i_wb_frn_avg_time;
  u16 i_wb_frn_history;
  struct list_head i_lru;
  struct list_head i_sb_list;
  struct list_head i_wb_list;
  union {
    struct hlist_head i_dentry;
    struct callback_head i_rcu;
  };
  atomic64_t i_version;
  atomic64_t i_sequence;
  atomic_t i_count;
  atomic_t i_dio_count;
  atomic_t i_writecount;
  atomic_t i_readcount;
  union {
    const struct file_operations *i_fop;
    void (*free_inode)(struct inode *);
  };
  struct file_lock_context *i_flctx;
  struct address_space i_data;
  union {
    struct list_head i_devices;
    int i_linklen;
  };
  union {
    struct pipe_inode_info *i_pipe;
    struct cdev *i_cdev;
    char *i_link;
    unsigned int i_dir_seq;
  };
  __u32 i_fsnotify_mask;
  struct fsnotify_mark_connector *i_fsnotify_marks;
  struct fscrypt_inode_info *i_crypt_info;
  void *i_private;
};

struct filename {
  const char *name;
  const char *uptr;
  atomic_t refcnt;
  struct audit_names *aname;
  const char iname[];
};

typedef long unsigned int uintptr_t;

typedef struct {
  atomic64_t refcnt;
} file_ref_t;

typedef unsigned int fmode_t;

struct mutex {
  atomic_long_t owner;
  raw_spinlock_t wait_lock;
  struct optimistic_spin_queue osq;
  struct list_head wait_list;
};

struct llist_node {
  struct llist_node *next;
};

struct file_ra_state {
  long unsigned int start;
  unsigned int size;
  unsigned int async_size;
  unsigned int ra_pages;
  unsigned int mmap_miss;
  loff_t prev_pos;
};

typedef struct {
  long unsigned int v;
} freeptr_t;

struct file {
  file_ref_t f_ref;
  spinlock_t f_lock;
  fmode_t f_mode;
  const struct file_operations *f_op;
  struct address_space *f_mapping;
  void *private_data;
  struct inode *f_inode;
  unsigned int f_flags;
  unsigned int f_iocb_flags;
  const struct cred *f_cred;
  struct path f_path;
  union {
    struct mutex f_pos_lock;
    u64 f_pipe;
  };
  loff_t f_pos;
  void *f_security;
  struct fown_struct *f_owner;
  errseq_t f_wb_err;
  errseq_t f_sb_err;
  struct hlist_head *f_ep;
  union {
    struct callback_head f_task_work;
    struct llist_node f_llist;
    struct file_ra_state f_ra;
    freeptr_t f_freeptr;
  };
};

typedef long unsigned int __kernel_ulong_t;

struct rlimit {
  __kernel_ulong_t rlim_cur;
  __kernel_ulong_t rlim_max;
};

struct linux_binprm {
  struct vm_area_struct *vma;
  long unsigned int vma_pages;
  long unsigned int argmin;
  struct mm_struct *mm;
  long unsigned int p;
  unsigned int have_execfd : 1;
  unsigned int execfd_creds : 1;
  unsigned int secureexec : 1;
  unsigned int point_of_no_return : 1;
  unsigned int comm_from_dentry : 1;
  unsigned int is_check : 1;
  struct file *executable;
  struct file *interpreter;
  struct file *file;
  struct cred *cred;
  int unsafe;
  unsigned int per_clear;
  int argc;
  int envc;
  const char *filename;
  const char *interp;
  const char *fdpath;
  unsigned int interp_flags;
  int execfd;
  long unsigned int loader;
  long unsigned int exec;
  struct rlimit rlim_stack;
  char buf[256];
};

#define SIGKILL 9

enum bpf_map_type {
  BPF_MAP_TYPE_UNSPEC = 0,
  BPF_MAP_TYPE_HASH = 1,
  BPF_MAP_TYPE_ARRAY = 2,
  BPF_MAP_TYPE_PROG_ARRAY = 3,
  BPF_MAP_TYPE_PERF_EVENT_ARRAY = 4,
  BPF_MAP_TYPE_PERCPU_HASH = 5,
  BPF_MAP_TYPE_PERCPU_ARRAY = 6,
  BPF_MAP_TYPE_STACK_TRACE = 7,
  BPF_MAP_TYPE_CGROUP_ARRAY = 8,
  BPF_MAP_TYPE_LRU_HASH = 9,
  BPF_MAP_TYPE_LRU_PERCPU_HASH = 10,
};

enum {
  BPF_ANY = 0,
  BPF_NOEXIST = 1,
  BPF_EXIST = 2,
};

enum {
  BPF_F_SKIP_FIELD_MASK = 255,
  BPF_F_USER_STACK = 256,
  BPF_F_FAST_STACK_CMP = 512,
  BPF_F_REUSE_STACKID = 1024,
  BPF_F_USER_BUILD_ID = 2048,
  BPF_F_KERNEL_STACK = 0,
};

struct ns_common {
  unsigned int inum;
};

struct uts_namespace {
  struct ns_common ns;
};

struct mnt_namespace {
  struct ns_common ns;
};

struct net {
  struct ns_common ns;
};

struct pid_namespace {
  struct ns_common ns;
};

struct nsproxy {
  struct uts_namespace *uts_ns;
  struct mnt_namespace *mnt_ns;
  struct pid_namespace *pid_ns_for_children;
  struct net *net_ns;
};

struct task_struct {
  struct nsproxy *nsproxy;
};

#endif
