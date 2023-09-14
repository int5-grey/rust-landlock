use crate::{uapi, Access, CompatError};

#[cfg(test)]
use std::convert::TryInto;
#[cfg(test)]
use strum::{EnumCount, IntoEnumIterator};
#[cfg(test)]
use strum_macros::{EnumCount as EnumCountMacro, EnumIter};

/// Version of the Landlock [ABI](https://en.wikipedia.org/wiki/Application_binary_interface).
///
/// `ABI` enables to get the features supported by a specific Landlock ABI.
/// For example, [`AccessFs::from_all(ABI::V1)`](Access::from_all)
/// gets all the file system access rights defined by the first version.
///
/// Without `ABI`, it would be hazardous to rely on the the full set of access flags
/// (e.g., `BitFlags::<AccessFs>::all()` or `BitFlags::ALL`),
/// a moving target that would change the semantics of your Landlock rule
/// when migrating to a newer version of this crate
/// (i.e. non-breaking change with new supported features).
/// This usage should then be considered indeterministic because requested features
/// (e.g., access rights)
/// could not be tied to the application source code.
///
/// Such `ABI` is also convenient to get the features supported by a specific Linux kernel
/// without relying on the kernel version (which may not be accessible or patched).
#[cfg_attr(
    test,
    derive(Debug, PartialEq, Eq, PartialOrd, EnumIter, EnumCountMacro)
)]
#[derive(Copy, Clone)]
#[non_exhaustive]
pub enum ABI {
    /// Kernel not supporting Landlock, either because it is not built with Landlock
    /// or Landlock is not enabled at boot.
    Unsupported = 0,
    /// First Landlock ABI, introduced with
    /// [Linux 5.13](https://git.kernel.org/stable/c/17ae69aba89dbfa2139b7f8024b757ab3cc42f59).
    V1 = 1,
    /// Second Landlock ABI, introduced with
    /// [Linux 5.19](https://git.kernel.org/stable/c/cb44e4f061e16be65b8a16505e121490c66d30d0).
    V2 = 2,
    /// Third Landlock ABI, introduced with
    /// [Linux 6.2](https://git.kernel.org/stable/c/299e2b1967578b1442128ba8b3e86ed3427d3651).
    V3 = 3,
}

impl ABI {
    // Must remain private to avoid inconsistent behavior by passing Ok(self) to a builder method,
    // e.g. to make it impossible to call ruleset.handle_fs(ABI::new_current()?)
    fn new_current() -> Self {
        ABI::from(unsafe {
            // Landlock ABI version starts at 1 but errno is only set for negative values.
            uapi::landlock_create_ruleset(
                std::ptr::null(),
                0,
                uapi::LANDLOCK_CREATE_RULESET_VERSION,
            )
        })
    }

    // There is no way to not publicly expose an implementation of an external trait such as
    // From<i32>.  See RFC https://github.com/rust-lang/rfcs/pull/2529
    fn from(value: i32) -> ABI {
        match value {
            // The only possible error values should be EOPNOTSUPP and ENOSYS, but let's interpret
            // all kind of errors as unsupported.
            n if n <= 0 => ABI::Unsupported,
            1 => ABI::V1,
            2 => ABI::V2,
            // Returns the greatest known ABI.
            _ => ABI::V3,
        }
    }

    #[cfg(test)]
    fn is_known(value: i32) -> bool {
        value > 0 && value < ABI::COUNT as i32
    }
}

#[test]
fn abi_from() {
    // EOPNOTSUPP (-95), ENOSYS (-38)
    for n in [-95, -38, -1, 0] {
        assert_eq!(ABI::from(n), ABI::Unsupported);
    }

    let mut last_i = 1;
    let mut last_abi = ABI::Unsupported;
    for (i, abi) in ABI::iter().enumerate() {
        last_i = i.try_into().unwrap();
        last_abi = abi;
        assert_eq!(ABI::from(last_i), last_abi);
    }

    assert_eq!(ABI::from(last_i + 1), last_abi);
    assert_eq!(ABI::from(9), last_abi);
}

#[test]
fn known_abi() {
    assert!(!ABI::is_known(-1));
    assert!(!ABI::is_known(0));
    assert!(!ABI::is_known(99));

    let mut last_i = -1;
    for (i, _) in ABI::iter().enumerate().skip(1) {
        last_i = i as i32;
        assert!(ABI::is_known(last_i));
    }
    assert!(!ABI::is_known(last_i + 1));
}

#[cfg(test)]
lazy_static! {
    static ref TEST_ABI: ABI = match std::env::var("LANDLOCK_CRATE_TEST_ABI") {
        Ok(s) => {
            let n = s.parse::<i32>().unwrap();
            if ABI::is_known(n) || n == 0 {
                ABI::from(n)
            } else {
                panic!("Unknown ABI: {n}");
            }
        }
        Err(std::env::VarError::NotPresent) => ABI::iter().last().unwrap(),
        Err(e) => panic!("Failed to read LANDLOCK_CRATE_TEST_ABI: {e}"),
    };
}

#[cfg(test)]
pub(crate) fn can_emulate(mock: ABI, partial_support: ABI, full_support: Option<ABI>) -> bool {
    mock < partial_support
        || mock <= *TEST_ABI
        || if let Some(full) = full_support {
            full <= *TEST_ABI
        } else {
            partial_support <= *TEST_ABI
        }
}

#[cfg(test)]
pub(crate) fn get_errno_from_landlock_status() -> Option<i32> {
    use std::io::Error;

    if unsafe {
        uapi::landlock_create_ruleset(std::ptr::null(), 0, uapi::LANDLOCK_CREATE_RULESET_VERSION)
    } < 0
    {
        match Error::last_os_error().raw_os_error() {
            // Returns ENOSYS when the kernel is not built with Landlock support,
            // or EOPNOTSUPP when Landlock is supported but disabled at boot time.
            ret @ Some(libc::ENOSYS | libc::EOPNOTSUPP) => ret,
            // Other values can only come from bogus seccomp filters or debug tampering.
            _ => unreachable!(),
        }
    } else {
        None
    }
}

#[test]
fn current_kernel_abi() {
    // Ensures that the tested Landlock ABI is the latest known version supported by the running
    // kernel.  If this test failed, you need set the LANDLOCK_CRATE_TEST_ABI environment variable
    // to the Landlock ABI version supported by your kernel.  With a missing variable, the latest
    // Landlock ABI version known by this crate is automatically set.
    // From Linux 5.13 to 5.18, you need to run: LANDLOCK_CRATE_TEST_ABI=1 cargo test
    assert_eq!(*TEST_ABI, ABI::new_current());
}

// CompatState is not public outside this crate.
/// Returned by ruleset builder.
#[cfg_attr(test, derive(Debug))]
#[derive(Copy, Clone, PartialEq, Eq)]
pub enum CompatState {
    /// Initial undefined state.
    Init,
    /// All requested restrictions are enforced.
    Full,
    /// Some requested restrictions are enforced, following a best-effort approach.
    Partial,
    /// The running system doesn't support Landlock.
    No,
    /// Final unsupported state.
    Dummy,
}

impl CompatState {
    fn update(&mut self, other: Self) {
        *self = match (*self, other) {
            (CompatState::Init, other) => other,
            (CompatState::Dummy, _) => CompatState::Dummy,
            (_, CompatState::Dummy) => CompatState::Dummy,
            (CompatState::No, CompatState::No) => CompatState::No,
            (CompatState::Full, CompatState::Full) => CompatState::Full,
            (_, _) => CompatState::Partial,
        }
    }
}

#[test]
fn compat_state_update_1() {
    let mut state = CompatState::Full;

    state.update(CompatState::Full);
    assert_eq!(state, CompatState::Full);

    state.update(CompatState::No);
    assert_eq!(state, CompatState::Partial);

    state.update(CompatState::Full);
    assert_eq!(state, CompatState::Partial);

    state.update(CompatState::Full);
    assert_eq!(state, CompatState::Partial);

    state.update(CompatState::No);
    assert_eq!(state, CompatState::Partial);

    state.update(CompatState::Dummy);
    assert_eq!(state, CompatState::Dummy);

    state.update(CompatState::Full);
    assert_eq!(state, CompatState::Dummy);
}

#[test]
fn compat_state_update_2() {
    let mut state = CompatState::Full;

    state.update(CompatState::Full);
    assert_eq!(state, CompatState::Full);

    state.update(CompatState::No);
    assert_eq!(state, CompatState::Partial);

    state.update(CompatState::Full);
    assert_eq!(state, CompatState::Partial);
}

#[cfg_attr(test, derive(Debug, PartialEq))]
#[derive(Clone)]
pub(crate) struct Compatibility {
    abi: ABI,
    pub(crate) level: Option<CompatLevel>,
    pub(crate) state: CompatState,
}

impl From<ABI> for Compatibility {
    fn from(abi: ABI) -> Self {
        Compatibility {
            abi,
            level: Default::default(),
            state: match abi {
                // Don't forces the state as Dummy because no_new_privs may still be legitimate.
                ABI::Unsupported => CompatState::No,
                _ => CompatState::Init,
            },
        }
    }
}

impl Compatibility {
    // Compatibility is a semi-opaque struct.
    #[allow(clippy::new_without_default)]
    pub(crate) fn new() -> Self {
        ABI::new_current().into()
    }

    pub(crate) fn update(&mut self, state: CompatState) {
        self.state.update(state);
    }

    pub(crate) fn abi(&self) -> ABI {
        self.abi
    }
}

pub(crate) mod private {
    use crate::CompatLevel;

    pub trait OptionCompatLevelMut {
        fn as_option_compat_level_mut(&mut self) -> &mut Option<CompatLevel>;
    }
}

/// Properly handles runtime unsupported features.
///
/// This guarantees consistent behaviors across crate users
/// and runtime kernels even if this crate get new features.
/// It eases backward compatibility and enables future-proofness.
///
/// Landlock is a security feature designed to help improve security of a running system
/// thanks to application developers.
/// To protect users as much as possible,
/// compatibility with the running system should then be handled in a best-effort way,
/// contrary to common system features.
/// In some circumstances
/// (e.g. applications carefully designed to only be run with a specific set of kernel features),
/// it may be required to error out if some of these features are not available
/// and will then not be enforced.
pub trait Compatible: Sized + private::OptionCompatLevelMut {
    /// To enable a best-effort security approach,
    /// Landlock features that are not supported by the running system
    /// are silently ignored by default,
    /// which is a sane choice for most use cases.
    /// However, on some rare circumstances,
    /// developers may want to have some guarantees that their applications
    /// will not run if a certain level of sandboxing is not possible.
    /// If we really want to error out when not all our requested requirements are met,
    /// then we can configure it with `set_compatibility()`.
    ///
    /// The `Compatible` trait is implemented for all object builders
    /// (e.g. [`Ruleset`](crate::Ruleset)).
    /// Such builders have a set of methods to incrementally build an object.
    /// These build methods rely on kernel features that may not be available at runtime.
    /// The `set_compatibility()` method enables to control the effect of
    /// the following build method calls starting after the `set_compatibility()` call.
    /// Such effect can be:
    /// * to silently ignore unsupported features
    ///   and continue building ([`CompatLevel::BestEffort`]);
    /// * to silently ignore unsupported features
    ///   and ignore the whole build ([`CompatLevel::SoftRequirement`]);
    /// * to return an error for any unsupported feature ([`CompatLevel::HardRequirement`]).
    ///
    /// Taking [`Ruleset`](crate::Ruleset) as an example,
    /// the [`handle_access()`](crate::RulesetAttr::handle_access()) build method
    /// returns a [`Result`] that can be [`Err(RulesetError)`](crate::RulesetError)
    /// with a nested [`CompatError`].
    /// Such error can only occur with a running Linux kernel not supporting the requested
    /// Landlock accesses *and* if the current compatibility level is
    /// [`CompatLevel::HardRequirement`].
    /// However, such error is not possible with [`CompatLevel::BestEffort`]
    /// nor [`CompatLevel::SoftRequirement`].
    ///
    /// The order of this call is important because
    /// it defines the behavior of the following build method calls that return a [`Result`].
    /// If `set_compatibility(CompatLevel::HardRequirement)` is called on an object,
    /// then a [`CompatError`] may be returned for the next method calls,
    /// until the next call to `set_compatibility()`.
    /// This enables to change the behavior of a set of build method calls,
    /// for instance to be sure that the sandbox will at least restrict some access rights.
    ///
    /// New objects inherit the compatibility configuration of their parents, if any.
    /// For instance, [`Ruleset::create()`](crate::Ruleset::create()) returns
    /// a [`RulesetCreated`](crate::RulesetCreated) object that inherits the
    /// `Ruleset`'s compatibility configuration.
    ///
    /// # Example with `SoftRequirement`
    ///
    /// Let's say an application legitimately needs to rename files between directories.
    /// Because of [previous Landlock limitations](https://docs.kernel.org/userspace-api/landlock.html#file-renaming-and-linking-abi-2),
    /// this was forbidden with the [first version of Landlock](ABI::V1),
    /// but it is now handled starting with the [second version](ABI::V2).
    /// For this use case, we only want the application to be sandboxed
    /// if we have the guarantee that it will not break a legitimate usage (i.e. rename files).
    /// We then create a ruleset which will either support file renaming
    /// (thanks to [`AccessFs::Refer`](crate::AccessFs::Refer)) or silently do nothing.
    ///
    /// ```
    /// use landlock::*;
    ///
    /// fn ruleset_handling_renames() -> Result<RulesetCreated, RulesetError> {
    ///     Ok(Ruleset::default()
    ///         // This ruleset must either handle the AccessFs::Refer right,
    ///         // or it must silently ignore the whole sandboxing.
    ///         .set_compatibility(CompatLevel::SoftRequirement)
    ///         .handle_access(AccessFs::Refer)?
    ///         // However, this ruleset may also handle other (future) access rights
    ///         // if they are supported by the running kernel.
    ///         .set_compatibility(CompatLevel::BestEffort)
    ///         .handle_access(AccessFs::from_all(ABI::V3))?
    ///         .create()?)
    /// }
    /// ```
    ///
    /// # Example with `HardRequirement`
    ///
    /// Security-dedicated applications may want to ensure that
    /// an untrusted software component is subject to a minimum of restrictions before launching it.
    /// In this case, we want to create a ruleset which will at least support
    /// all restrictions provided by the [first version of Landlock](ABI::V1),
    /// and opportunistically handle restrictions supported by newer kernels.
    ///
    /// ```
    /// use landlock::*;
    ///
    /// fn ruleset_fragile() -> Result<RulesetCreated, RulesetError> {
    ///     Ok(Ruleset::default()
    ///         // This ruleset must either handle at least all accesses defined by
    ///         // the first Landlock version (e.g. AccessFs::WriteFile),
    ///         // or the following handle_access() call must return a wrapped
    ///         // AccessError<AccessFs>::Incompatible error.
    ///         .set_compatibility(CompatLevel::HardRequirement)
    ///         .handle_access(AccessFs::from_all(ABI::V1))?
    ///         // However, this ruleset may also handle new access rights
    ///         // (e.g. AccessFs::Refer defined by the second version of Landlock)
    ///         // if they are supported by the running kernel,
    ///         // but without returning any error otherwise.
    ///         .set_compatibility(CompatLevel::BestEffort)
    ///         .handle_access(AccessFs::from_all(ABI::V2))?
    ///         .create()?)
    /// }
    /// ```
    fn set_compatibility(mut self, level: CompatLevel) -> Self {
        *self.as_option_compat_level_mut() = Some(level);
        self
    }

    /// Cf. [`set_compatibility()`](Compatible::set_compatibility()):
    ///
    /// - `set_best_effort(true)` translates to `set_compatibility(CompatLevel::BestEffort)`.
    ///
    /// - `set_best_effort(false)` translates to `set_compatibility(CompatLevel::HardRequirement)`.
    #[deprecated(note = "Use set_compatibility() instead")]
    fn set_best_effort(self, best_effort: bool) -> Self
    where
        Self: Sized,
    {
        self.set_compatibility(match best_effort {
            true => CompatLevel::BestEffort,
            false => CompatLevel::HardRequirement,
        })
    }
}

#[test]
#[allow(deprecated)]
fn deprecated_set_best_effort() {
    use crate::{CompatLevel, Compatible, Ruleset};

    assert_eq!(
        Ruleset::default().set_best_effort(true).compat,
        Ruleset::default()
            .set_compatibility(CompatLevel::BestEffort)
            .compat
    );
    assert_eq!(
        Ruleset::default().set_best_effort(false).compat,
        Ruleset::default()
            .set_compatibility(CompatLevel::HardRequirement)
            .compat
    );
}

/// See the [`Compatible`] documentation.
#[cfg_attr(test, derive(EnumIter))]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CompatLevel {
    /// Takes into account the build requests if they are supported by the running system,
    /// or silently ignores them otherwise.
    /// Never returns a compatibility error.
    #[default]
    BestEffort,
    /// Takes into account the build requests if they are supported by the running system,
    /// or silently ignores the whole build object otherwise.
    /// Never returns a compatibility error.
    /// If not supported,
    /// the call to [`RulesetCreated::restrict_self()`](crate::RulesetCreated::restrict_self())
    /// will return a
    /// [`RestrictionStatus { ruleset: RulesetStatus::NotEnforced, no_new_privs: false, }`](crate::RestrictionStatus).
    SoftRequirement,
    /// Takes into account the build requests if they are supported by the running system,
    /// or returns a compatibility error otherwise ([`CompatError`]).
    HardRequirement,
}

impl From<Option<CompatLevel>> for CompatLevel {
    fn from(opt: Option<CompatLevel>) -> Self {
        match opt {
            None => CompatLevel::default(),
            Some(ref level) => *level,
        }
    }
}

// TailoredCompatLevel could be replaced with AsMut<Option<CompatLevel>>, but only traits defined
// in the current crate can be implemented for types defined outside of the crate.  Furthermore it
// provides a default implementation which is handy for types such as BitFlags.
pub trait TailoredCompatLevel {
    fn tailored_compat_level<L>(&mut self, parent_level: L) -> CompatLevel
    where
        L: Into<CompatLevel>,
    {
        parent_level.into()
    }
}

impl<T> TailoredCompatLevel for T
where
    Self: Compatible,
{
    // Every Compatible trait implementation returns its own compatibility level, if set.
    fn tailored_compat_level<L>(&mut self, parent_level: L) -> CompatLevel
    where
        L: Into<CompatLevel>,
    {
        // Using a mutable reference is not required but it makes the code simpler (no double AsRef
        // implementations for each Compatible types), and more importantly it guarantees
        // consistency with Compatible::set_compatibility().
        match self.as_option_compat_level_mut() {
            None => parent_level.into(),
            // Returns the most constrained compatibility level.
            Some(ref level) => parent_level.into().max(*level),
        }
    }
}

#[test]
fn tailored_compat_level() {
    use crate::{AccessFs, PathBeneath, PathFd};

    fn new_path(level: CompatLevel) -> PathBeneath<PathFd> {
        PathBeneath::new(PathFd::new("/").unwrap(), AccessFs::Execute).set_compatibility(level)
    }

    for parent_level in CompatLevel::iter() {
        assert_eq!(
            new_path(CompatLevel::BestEffort).tailored_compat_level(parent_level),
            parent_level
        );
        assert_eq!(
            new_path(CompatLevel::HardRequirement).tailored_compat_level(parent_level),
            CompatLevel::HardRequirement
        );
    }

    assert_eq!(
        new_path(CompatLevel::SoftRequirement).tailored_compat_level(CompatLevel::SoftRequirement),
        CompatLevel::SoftRequirement
    );

    for child_level in CompatLevel::iter() {
        assert_eq!(
            new_path(child_level).tailored_compat_level(CompatLevel::BestEffort),
            child_level
        );
        assert_eq!(
            new_path(child_level).tailored_compat_level(CompatLevel::HardRequirement),
            CompatLevel::HardRequirement
        );
    }
}

// CompatResult is useful because we don't want to duplicate objects (potentially wrapping a file
// descriptor), and we may not have compatibility errors for some objects.  TryCompat::try_compat()
// is responsible to either take T or CompatError<A> according to the compatibility level.
//
// CompatResult is not public outside this crate.
pub enum CompatResult<T, A>
where
    T: TryCompat<A>,
    A: Access,
{
    // Fully matches the request.
    Full(T),
    // Partially matches the request.
    Partial(T, CompatError<A>),
    // Doesn't matches the request.
    No(CompatError<A>),
}

// TryCompat is not public outside this crate.
pub trait TryCompat<A>
where
    Self: Sized + TailoredCompatLevel,
    A: Access,
{
    fn try_compat_inner(self, abi: ABI) -> Result<CompatResult<Self, A>, CompatError<A>>;

    // Default implementation for objects without children.
    //
    // If returning something other than Ok(Some(self)), the implementation must use its own
    // compatibility level, if any, with self.tailored_compat_level(default_compat_level), and pass
    // it with the abi and compat_state to each child.try_compat().  See PathBeneath implementation
    // and the self.allowed_access.try_compat() call.
    fn try_compat_children<L>(
        self,
        _abi: ABI,
        _parent_level: L,
        _compat_state: &mut CompatState,
    ) -> Result<Option<Self>, CompatError<A>>
    where
        L: Into<CompatLevel>,
    {
        Ok(Some(self))
    }

    // Update compat_state and return an error according to try_compat_*() error, or to the
    // compatibility level, i.e. either route compatible object or error.
    fn try_compat<L>(
        mut self,
        abi: ABI,
        parent_level: L,
        compat_state: &mut CompatState,
    ) -> Result<Option<Self>, CompatError<A>>
    where
        L: Into<CompatLevel>,
    {
        let compat_level = self.tailored_compat_level(parent_level);
        let new_self = match self.try_compat_children(abi, compat_level, compat_state)? {
            Some(n) => n,
            None => return Ok(None),
        };
        match new_self.try_compat_inner(abi) {
            Ok(CompatResult::Full(new_self)) => {
                compat_state.update(CompatState::Full);
                Ok(Some(new_self))
            }
            Ok(CompatResult::Partial(new_self, error)) => match compat_level {
                CompatLevel::BestEffort => {
                    compat_state.update(CompatState::Partial);
                    Ok(Some(new_self))
                }
                CompatLevel::SoftRequirement => {
                    compat_state.update(CompatState::Dummy);
                    Ok(None)
                }
                CompatLevel::HardRequirement => {
                    compat_state.update(CompatState::Dummy);
                    Err(error)
                }
            },
            Ok(CompatResult::No(error)) => match compat_level {
                CompatLevel::BestEffort => {
                    compat_state.update(CompatState::No);
                    Ok(None)
                }
                CompatLevel::SoftRequirement => {
                    compat_state.update(CompatState::Dummy);
                    Ok(None)
                }
                CompatLevel::HardRequirement => {
                    compat_state.update(CompatState::Dummy);
                    Err(error)
                }
            },
            Err(e) => {
                // Safeguard to help for test consistency.
                compat_state.update(CompatState::Dummy);
                Err(e)
            }
        }
    }
}
