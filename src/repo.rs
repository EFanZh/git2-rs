use std::c_str::CString;
use std::kinds::marker;
use std::mem;
use std::str;
use libc::{c_int, c_uint, c_char, size_t, c_void};

use {raw, Revspec, Error, init, Object, RepositoryState, Remote};
use {StringArray, ResetType, Signature, Reference, References, Submodule};
use {Branches, BranchType, Index, Config, Oid, Blob, Branch, Commit, Tree};
use {ObjectKind};
use build::RepoBuilder;

/// An owned git repository, representing all state associated with the
/// underlying filesystem.
///
/// This structure corresponds to a `git_repository` in libgit2. Many other
/// types in git2-rs are derivative from this structure and are attached to its
/// lifetime.
///
/// When a repository goes out of scope it is freed in memory but not deleted
/// from the filesystem.
pub struct Repository {
    raw: *mut raw::git_repository,
    marker: marker::NoSync,
}

impl Repository {
    /// Attempt to open an already-existing repository at `path`.
    ///
    /// The path can point to either a normal or bare repository.
    pub fn open(path: &Path) -> Result<Repository, Error> {
        init();
        let mut ret = 0 as *mut raw::git_repository;
        unsafe {
            try_call!(raw::git_repository_open(&mut ret, path.to_c_str()));
        }
        Ok(unsafe { Repository::from_raw(ret) })
    }

    /// Internal init, so that a boolean arg isn't exposed to userland.
    fn init_(path: &Path, bare: bool) -> Result<Repository, Error> {
        init();
        let mut ret = 0 as *mut raw::git_repository;
        unsafe {
            try_call!(raw::git_repository_init(&mut ret, path.to_c_str(),
                                               bare as c_uint));
        }
        Ok(unsafe { Repository::from_raw(ret) })
    }

    /// Creates a new repository in the specified folder.
    ///
    /// The folder must exist prior to invoking this function.
    pub fn init(path: &Path) -> Result<Repository, Error> {
        Repository::init_(path, false)
    }

    /// Creates a new `--bare` repository in the specified folder.
    ///
    /// The folder must exist prior to invoking this function.
    pub fn init_bare(path: &Path) -> Result<Repository, Error> {
        Repository::init_(path, true)
    }

    /// Clone a remote repository.
    ///
    /// See the `RepoBuilder` struct for more information. This function will
    /// delegate to a fresh `RepoBuilder`
    pub fn clone(url: &str, into: &Path) -> Result<Repository, Error> {
        ::init();
        RepoBuilder::new().clone(url, into)
    }

    /// Create a repository from the raw underlying pointer.
    ///
    /// This function will take ownership of the pointer specified.
    pub unsafe fn from_raw(ptr: *mut raw::git_repository) -> Repository {
        Repository {
            raw: ptr,
            marker: marker::NoSync,
        }
    }

    /// Execute a rev-parse operation against the `spec` listed.
    ///
    /// The resulting revision specification is returned, or an error is
    /// returned if one occurs.
    pub fn revparse(&self, spec: &str) -> Result<Revspec, Error> {
        let mut raw = raw::git_revspec {
            from: 0 as *mut _,
            to: 0 as *mut _,
            flags: raw::git_revparse_mode_t::empty(),
        };
        unsafe {
            try_call!(raw::git_revparse(&mut raw, self.raw, spec.to_c_str()));
        }

        if raw.flags.contains(raw::GIT_REVPARSE_SINGLE) {
            assert!(raw.to.is_null());
            let obj = unsafe { Object::from_raw(self, raw.from) };
            Ok(Revspec::from_objects(Some(obj), None))
        } else {
            fail!()
        }
    }

    /// Find a single object, as specified by a revision string.
    pub fn revparse_single(&self, spec: &str) -> Result<Object, Error> {
        let mut obj = 0 as *mut raw::git_object;
        unsafe {
            try_call!(raw::git_revparse_single(&mut obj, self.raw,
                                               spec.to_c_str()));
        }
        assert!(!obj.is_null());
        Ok(unsafe { Object::from_raw(self, obj) })
    }

    /// Tests whether this repository is a bare repository or not.
    pub fn is_bare(&self) -> bool {
        unsafe { raw::git_repository_is_bare(self.raw) == 1 }
    }

    /// Tests whether this repository is a shallow clone.
    pub fn is_shallow(&self) -> bool {
        unsafe { raw::git_repository_is_shallow(self.raw) == 1 }
    }

    /// Tests whether this repository is empty.
    pub fn is_empty(&self) -> Result<bool, Error> {
        let empty = unsafe {
            try_call!(raw::git_repository_is_empty(self.raw))
        };
        Ok(empty == 1)
    }

    /// Returns the path to the `.git` folder for normal repositories or the
    /// repository itself for bare repositories.
    pub fn path(&self) -> Path {
        unsafe {
            let ptr = raw::git_repository_path(self.raw);
            assert!(!ptr.is_null());
            Path::new(CString::new(ptr, false).as_bytes_no_nul())
        }
    }

    /// Returns the current state of this repository
    pub fn state(&self) -> RepositoryState {
        let state = unsafe { raw::git_repository_state(self.raw) };
        macro_rules! check( ($($raw:ident => $real:ident),*) => (
            $(if state == raw::$raw as c_int { super::$real }) else *
            else {
                fail!("unknown repository state: {}", state)
            }
        ) )

        check!(
            GIT_REPOSITORY_STATE_NONE => Clean,
            GIT_REPOSITORY_STATE_MERGE => Merge,
            GIT_REPOSITORY_STATE_REVERT => Revert,
            GIT_REPOSITORY_STATE_CHERRYPICK => CherryPick,
            GIT_REPOSITORY_STATE_BISECT => Bisect,
            GIT_REPOSITORY_STATE_REBASE => Rebase,
            GIT_REPOSITORY_STATE_REBASE_INTERACTIVE => RebaseInteractive,
            GIT_REPOSITORY_STATE_REBASE_MERGE => RebaseMerge,
            GIT_REPOSITORY_STATE_APPLY_MAILBOX => ApplyMailbox,
            GIT_REPOSITORY_STATE_APPLY_MAILBOX_OR_REBASE => ApplyMailboxOrRebase
        )
    }

    /// Get the path of the working directory for this repository.
    ///
    /// If this repository is bare, then `None` is returned.
    pub fn workdir(&self) -> Option<Path> {
        unsafe {
            let ptr = raw::git_repository_workdir(self.raw);
            if ptr.is_null() {
                None
            } else {
                Some(Path::new(CString::new(ptr, false).as_bytes_no_nul()))
            }
        }
    }

    /// Get the currently active namespace for this repository.
    ///
    /// If there is no namespace, or the namespace is not a valid utf8 string,
    /// `None` is returned.
    pub fn namespace(&self) -> Option<&str> {
        self.namespace_bytes().and_then(str::from_utf8)
    }

    /// Get the currently active namespace for this repository as a byte array.
    ///
    /// If there is no namespace, `None` is returned.
    pub fn namespace_bytes(&self) -> Option<&[u8]> {
        unsafe { ::opt_bytes(self, raw::git_repository_get_namespace(self.raw)) }
    }

    /// List all remotes for a given repository
    pub fn remotes(&self) -> Result<StringArray, Error> {
        let mut arr = raw::git_strarray {
            strings: 0 as *mut *mut c_char,
            count: 0,
        };
        unsafe {
            try_call!(raw::git_remote_list(&mut arr, self.raw));
        }
        Ok(unsafe { StringArray::from_raw(arr) })
    }

    /// Get the information for a particular remote
    pub fn find_remote(&self, name: &str) -> Result<Remote, Error> {
        let mut ret = 0 as *mut raw::git_remote;
        unsafe {
            try_call!(raw::git_remote_load(&mut ret, self.raw, name.to_c_str()));
            Ok(Remote::from_raw(self, ret))
        }
    }

    /// Add a remote with the default fetch refspec to the repository's
    /// configuration.
    pub fn remote(&self, name: &str, url: &str) -> Result<Remote, Error> {
        let mut ret = 0 as *mut raw::git_remote;
        unsafe {
            try_call!(raw::git_remote_create(&mut ret, self.raw,
                                             name.to_c_str(), url.to_c_str()));
            Ok(Remote::from_raw(self, ret))
        }
    }

    /// Create an anonymous remote
    ///
    /// Create a remote with the given url and refspec in memory. You can use
    /// this when you have a URL instead of a remote's name. Note that anonymous
    /// remotes cannot be converted to persisted remotes.
    pub fn remote_anonymous(&self, url: &str,
                            fetch: &str) -> Result<Remote, Error> {
        let mut ret = 0 as *mut raw::git_remote;
        unsafe {
            try_call!(raw::git_remote_create_anonymous(&mut ret, self.raw,
                                                       url.to_c_str(),
                                                       fetch.to_c_str()));
            Ok(Remote::from_raw(self, ret))
        }
    }

    /// Get the underlying raw repository
    pub fn raw(&self) -> *mut raw::git_repository { self.raw }

    /// Sets the current head to the specified object and optionally resets
    /// the index and working tree to match.
    ///
    /// A soft reset means the head will be moved to the commit.
    ///
    /// A mixed reset will trigger a soft reset, plus the index will be
    /// replaced with the content of the commit tree.
    ///
    /// A hard reset will trigger a mixed reset and the working directory will
    /// be replaced with the content of the index. (Untracked and ignored files
    /// will be left alone, however.)
    pub fn reset<'a>(&'a self, target: &Object<'a>, kind: ResetType,
                     sig: Option<&Signature>, msg: Option<&str>)
                     -> Result<(), Error> {
        unsafe {
            try_call!(raw::git_reset(self.raw, target.raw(), kind,
                                     sig.map(|s| s.raw()).unwrap_or(0 as *mut _),
                                     msg.map(|s| s.to_c_str())));
        }
        Ok(())
    }

    /// Updates some entries in the index from the target commit tree.
    ///
    /// The scope of the updated entries is determined by the paths being
    /// in the iterator provided.
    ///
    /// Passing a `None` target will result in removing entries in the index
    /// matching the provided pathspecs.
    pub fn reset_default<'a,
                         T: ToCStr,
                         I: Iterator<T>>(&'a self,
                                         target: Option<&Object<'a>>,
                                         paths: I) -> Result<(), Error> {
        let v = paths.map(|t| t.to_c_str()).collect::<Vec<CString>>();
        let v2 = v.iter().map(|v| v.as_ptr()).collect::<Vec<*const c_char>>();
        let mut arr = raw::git_strarray {
            strings: v2.as_ptr() as *mut _,
            count: v2.len() as size_t,
        };
        let target = target.map(|t| t.raw()).unwrap_or(0 as *mut _);

        unsafe {
            try_call!(raw::git_reset_default(self.raw, target, &mut arr));
        }
        Ok(())
    }

    /// Retrieve and resolve the reference pointed at by HEAD.
    pub fn head(&self) -> Result<Reference, Error> {
        let mut ret = 0 as *mut raw::git_reference;
        unsafe {
            try_call!(raw::git_repository_head(&mut ret, self.raw));
            Ok(Reference::from_raw(self, ret))
        }
    }

    /// Create an iterator for the repo's references
    pub fn references(&self) -> Result<References, Error> {
        let mut ret = 0 as *mut raw::git_reference_iterator;
        unsafe {
            try_call!(raw::git_reference_iterator_new(&mut ret, self.raw));
            Ok(References::from_raw(self, ret))
        }
    }

    /// Create an iterator for the repo's references that match the specified
    /// glob
    pub fn references_glob(&self, glob: &str) -> Result<References, Error> {
        let mut ret = 0 as *mut raw::git_reference_iterator;
        unsafe {
            try_call!(raw::git_reference_iterator_glob_new(&mut ret, self.raw,
                                                           glob.to_c_str()));
            Ok(References::from_raw(self, ret))
        }
    }

    /// Load all submodules for this repository and return them.
    pub fn submodules(&self) -> Result<Vec<Submodule>, Error> {
        struct Data<'a, 'b:'a> {
            repo: &'b Repository,
            ret: &'a mut Vec<Submodule<'b>>,
        }
        let mut ret = Vec::new();

        unsafe {
            let mut data = Data {
                repo: self,
                ret: &mut ret,
            };
            try_call!(raw::git_submodule_foreach(self.raw, append,
                                                 &mut data as *mut _
                                                           as *mut c_void));
        }

        return Ok(ret);

        extern fn append(_repo: *mut raw::git_submodule,
                         name: *const c_char,
                         data: *mut c_void) -> c_int {
            unsafe {
                let data = &mut *(data as *mut Data);
                let mut raw = 0 as *mut raw::git_submodule;
                let rc = raw::git_submodule_lookup(&mut raw, data.repo.raw(),
                                                   name);
                assert_eq!(rc, 0);
                data.ret.push(Submodule::from_raw(data.repo, raw));
            }
            0
        }
    }

    /// Create an iterator which loops over the requested branches.
    pub fn branches(&self, filter: Option<BranchType>) -> Result<Branches, Error> {
        let mut raw = 0 as *mut raw::git_branch_iterator;
        unsafe {
            try_call!(raw::git_branch_iterator_new(&mut raw, self.raw(), filter));
            Ok(Branches::from_raw(self, raw))
        }
    }

    /// Get the Index file for this repository.
    ///
    /// If a custom index has not been set, the default index for the repository
    /// will be returned (the one located in .git/index).
    pub fn index(&self) -> Result<Index, Error> {
        let mut raw = 0 as *mut raw::git_index;
        unsafe {
            try_call!(raw::git_repository_index(&mut raw, self.raw()));
            Ok(Index::from_raw(raw))
        }
    }

    /// Get the configuration file for this repository.
    ///
    /// If a configuration file has not been set, the default config set for the
    /// repository will be returned, including global and system configurations
    /// (if they are available).
    pub fn config(&self) -> Result<Config, Error> {
        let mut raw = 0 as *mut raw::git_config;
        unsafe {
            try_call!(raw::git_repository_config(&mut raw, self.raw()));
            Ok(Config::from_raw(raw))
        }
    }

    /// Write an in-memory buffer to the ODB as a blob.
    ///
    /// The Oid returned can in turn be passed to `find_blob` to get a handle to
    /// the blob.
    pub fn blob(&self, data: &[u8]) -> Result<Oid, Error> {
        let mut raw = raw::git_oid { id: [0, ..raw::GIT_OID_RAWSZ] };
        unsafe {
            let ptr = data.as_ptr() as *const c_void;
            let len = data.len() as size_t;
            try_call!(raw::git_blob_create_frombuffer(&mut raw, self.raw(),
                                                      ptr, len));
            Ok(Oid::from_raw(&raw))
        }
    }

    /// Read a file from the filesystem and write its content to the Object
    /// Database as a loose blob
    ///
    /// The Oid returned can in turn be passed to `find_blob` to get a handle to
    /// the blob.
    pub fn blob_path(&self, path: &Path) -> Result<Oid, Error> {
        let mut raw = raw::git_oid { id: [0, ..raw::GIT_OID_RAWSZ] };
        unsafe {
            try_call!(raw::git_blob_create_fromdisk(&mut raw, self.raw(),
                                                    path.to_c_str()));
            Ok(Oid::from_raw(&raw))
        }
    }

    /// Lookup a reference to one of the objects in a repository.
    pub fn find_blob(&self, oid: Oid) -> Result<Blob, Error> {
        let mut raw = 0 as *mut raw::git_blob;
        unsafe {
            try_call!(raw::git_blob_lookup(&mut raw, self.raw(), oid.raw()));
            Ok(Blob::from_raw(self, raw))
        }
    }

    /// Create a new branch pointing at a target commit
    ///
    /// A new direct reference will be created pointing to this target commit.
    /// If `force` is true and a reference already exists with the given name,
    /// it'll be replaced.
    pub fn branch<'a>(&'a self,
                      branch_name: &str,
                      target: &Commit<'a>,
                      force: bool,
                      signature: Option<&Signature>,
                      log_message: &str) -> Result<Branch<'a>, Error> {
        let mut raw = 0 as *mut raw::git_reference;
        unsafe {
            try_call!(raw::git_branch_create(&mut raw,
                                             self.raw(),
                                             branch_name.to_c_str(),
                                             &*target.raw(),
                                             force,
                                             &*signature.map(|s| s.raw())
                                                        .unwrap_or(0 as *mut _),
                                             log_message.to_c_str()));
            Ok(Branch::wrap(Reference::from_raw(self, raw)))
        }
    }

    /// Lookup a branch by its name in a repository.
    pub fn find_branch(&self, name: &str, branch_type: BranchType)
                       -> Result<Branch, Error> {
        let mut ret = 0 as *mut raw::git_reference;
        unsafe {
            try_call!(raw::git_branch_lookup(&mut ret, self.raw(),
                                             name.to_c_str(), branch_type));
            Ok(Branch::wrap(Reference::from_raw(self, ret)))
        }
    }

    /// Create new commit in the repository
    ///
    /// If the `update_ref` is not `None`, name of the reference that will be
    /// updated to point to this commit. If the reference is not direct, it will
    /// be resolved to a direct reference. Use "HEAD" to update the HEAD of the
    /// current branch and make it point to this commit. If the reference
    /// doesn't exist yet, it will be created. If it does exist, the first
    /// parent must be the tip of this branch.
    pub fn commit<'a>(&'a self,
                      update_ref: Option<&str>,
                      author: &Signature,
                      committer: &Signature,
                      message: &str,
                      tree: &Tree<'a>,
                      parents: &[&Commit<'a>]) -> Result<Oid, Error> {
        let mut raw = raw::git_oid { id: [0, ..raw::GIT_OID_RAWSZ] };
        let parent_ptrs: Vec<*const raw::git_commit> =  parents.iter().map(|p| {
            p.raw() as *const raw::git_commit
        }).collect();
        unsafe {
            try_call!(raw::git_commit_create(&mut raw,
                                             self.raw(),
                                             update_ref.map(|s| s.to_c_str()),
                                             &*author.raw(),
                                             &*committer.raw(),
                                             0 as *const c_char,
                                             message.to_c_str(),
                                             &*tree.raw(),
                                             parents.len() as size_t,
                                             parent_ptrs.as_ptr()));
            Ok(Oid::from_raw(&raw))
        }
    }


    /// Lookup a reference to one of the commits in a repository.
    pub fn find_commit(&self, oid: Oid) -> Result<Commit, Error> {
        let mut raw = 0 as *mut raw::git_commit;
        unsafe {
            try_call!(raw::git_commit_lookup(&mut raw, self.raw(), oid.raw()));
            Ok(Commit::from_raw(self, raw))
        }
    }

    /// Lookup a reference to one of the objects in a repository.
    pub fn find_object(&self, oid: Oid,
                       kind: Option<ObjectKind>) -> Result<Object, Error> {
        let mut raw = 0 as *mut raw::git_object;
        unsafe {
            try_call!(raw::git_object_lookup(&mut raw, self.raw(), oid.raw(),
                                             kind));
            Ok(Object::from_raw(self, raw))
        }
    }

    /// Create a new direct reference.
    ///
    /// This function will return an error if a reference already exists with
    /// the given name unless force is true, in which case it will be
    /// overwritten.
    pub fn reference(&self, name: &str, id: Oid, force: bool,
                     sig: Option<&Signature>,
                     log_message: &str) -> Result<Reference, Error> {
        let mut raw = 0 as *mut raw::git_reference;
        unsafe {
            try_call!(raw::git_reference_create(&mut raw, self.raw(),
                                                name.to_c_str(),
                                                &*id.raw(), force,
                                                &*sig.map(|s| s.raw())
                                                     .unwrap_or(0 as *mut _),
                                                log_message.to_c_str()));
            Ok(Reference::from_raw(self, raw))
        }
    }

    /// Create a new symbolic reference.
    ///
    /// This function will return an error if a reference already exists with
    /// the given name unless force is true, in which case it will be
    /// overwritten.
    pub fn reference_symbolic(&self, name: &str, target: &str,
                              force: bool, sig: Option<&Signature>,
                              log_message: &str)
                              -> Result<Reference, Error> {
        let mut raw = 0 as *mut raw::git_reference;
        unsafe {
            try_call!(raw::git_reference_symbolic_create(&mut raw, self.raw(),
                                                         name.to_c_str(),
                                                         target.to_c_str(),
                                                         force,
                                                         &*sig.map(|s| s.raw())
                                                              .unwrap_or(0 as *mut _),
                                                         log_message.to_c_str()));
            Ok(Reference::from_raw(self, raw))
        }
    }

    /// Lookup a reference to one of the objects in a repository.
    pub fn find_reference(&self, name: &str) -> Result<Reference, Error> {
        let mut raw = 0 as *mut raw::git_reference;
        unsafe {
            try_call!(raw::git_reference_lookup(&mut raw, self.raw(),
                                                name.to_c_str()));
            Ok(Reference::from_raw(self, raw))
        }
    }

    /// Lookup a reference by name and resolve immediately to OID.
    ///
    /// This function provides a quick way to resolve a reference name straight
    /// through to the object id that it refers to. This avoids having to
    /// allocate or free any `Reference` objects for simple situations.
    pub fn refname_to_id(&self, name: &str) -> Result<Oid, Error> {
        let mut ret: raw::git_oid = unsafe { mem::zeroed() };
        unsafe {
            try_call!(raw::git_reference_name_to_id(&mut ret, self.raw(),
                                                    name.to_c_str()));
            Ok(Oid::from_raw(&ret))
        }
    }

    /// Create a new action signature with default user and now timestamp.
    ///
    /// This looks up the user.name and user.email from the configuration and
    /// uses the current time as the timestamp, and creates a new signature
    /// based on that information. It will return `NotFound` if either the
    /// user.name or user.email are not set.
    pub fn signature(&self) -> Result<Signature<'static>, Error> {
        let mut ret = 0 as *mut raw::git_signature;
        unsafe {
            try_call!(raw::git_signature_default(&mut ret, self.raw()));
            Ok(Signature::from_raw(ret))
        }
    }

    /// Set up a new git submodule for checkout.
    ///
    /// This does "git submodule add" up to the fetch and checkout of the
    /// submodule contents. It preps a new submodule, creates an entry in
    /// `.gitmodules` and creates an empty initialized repository either at the
    /// given path in the working directory or in `.git/modules` with a gitlink
    /// from the working directory to the new repo.
    ///
    /// To fully emulate "git submodule add" call this function, then `open()`
    /// the submodule repo and perform the clone step as needed. Lastly, call
    /// `finalize()` to wrap up adding the new submodule and `.gitmodules` to
    /// the index to be ready to commit.
    pub fn submodule(&self, url: &str, path: &Path,
                     use_gitlink: bool) -> Result<Submodule, Error> {
        let mut raw = 0 as *mut raw::git_submodule;
        unsafe {
            try_call!(raw::git_submodule_add_setup(&mut raw, self.raw(),
                                                   url.to_c_str(),
                                                   path.to_c_str(),
                                                   use_gitlink));
            Ok(Submodule::from_raw(self, raw))
        }
    }

    /// Lookup submodule information by name or path.
    ///
    /// Given either the submodule name or path (they are usually the same),
    /// this returns a structure describing the submodule.
    pub fn find_submodule(&self, name: &str) -> Result<Submodule, Error> {
        let mut raw = 0 as *mut raw::git_submodule;
        unsafe {
            try_call!(raw::git_submodule_lookup(&mut raw, self.raw(),
                                                name.to_c_str()));
            Ok(Submodule::from_raw(self, raw))
        }
    }

    /// Lookup a reference to one of the objects in a repository.
    pub fn find_tree(&self, oid: Oid) -> Result<Tree, Error> {
        let mut raw = 0 as *mut raw::git_tree;
        unsafe {
            try_call!(raw::git_tree_lookup(&mut raw, self.raw(), oid.raw()));
            Ok(Tree::from_raw(self, raw))
        }
    }
}

#[unsafe_destructor]
impl Drop for Repository {
    fn drop(&mut self) {
        unsafe { raw::git_repository_free(self.raw) }
    }
}

#[cfg(test)]
mod tests {
    use std::io::TempDir;
    use {Repository};

    #[test]
    fn smoke_init() {
        let td = TempDir::new("test").unwrap();
        let path = td.path();

        let repo = Repository::init(path).unwrap();
        assert!(!repo.is_bare());
    }

    #[test]
    fn smoke_init_bare() {
        let td = TempDir::new("test").unwrap();
        let path = td.path();

        let repo = Repository::init_bare(path).unwrap();
        assert!(repo.is_bare());
        assert!(repo.namespace().is_none());
    }

    #[test]
    fn smoke_open() {
        let td = TempDir::new("test").unwrap();
        let path = td.path();
        Repository::init(td.path()).unwrap();
        let repo = Repository::open(path).unwrap();
        assert!(!repo.is_bare());
        assert!(!repo.is_shallow());
        assert!(repo.is_empty().unwrap());
        assert!(repo.path() == td.path().join(".git"));
        assert_eq!(repo.state(), ::Clean);
    }

    #[test]
    fn smoke_open_bare() {
        let td = TempDir::new("test").unwrap();
        let path = td.path();
        Repository::init_bare(td.path()).unwrap();

        let repo = Repository::open(path).unwrap();
        assert!(repo.is_bare());
        assert!(repo.path() == *td.path());
    }

    #[test]
    fn smoke_revparse() {
        let (_td, repo) = ::test::repo_init();
        let rev = repo.revparse("HEAD").unwrap();
        assert!(rev.to().is_none());
        let from = rev.from().unwrap();
        assert!(rev.from().is_some());

        assert_eq!(repo.revparse_single("HEAD").unwrap().id(), from.id());
        let obj = repo.find_object(from.id(), None).unwrap().clone();
        obj.peel(::Any).unwrap();
        obj.short_id().unwrap();
        let sig = repo.signature().unwrap();
        repo.reset(&obj, ::Hard, None, None).unwrap();
        repo.reset(&obj, ::Soft, Some(&sig), Some("foo")).unwrap();
    }
}
