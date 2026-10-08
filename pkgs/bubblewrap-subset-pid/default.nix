{ bubblewrap }:

# Use the caller's bubblewrap package, including its upstream patches.
bubblewrap.overrideAttrs (old: {
  pname = "bubblewrap-subset-pid";
  postPatch = (old.postPatch or "") + ''
    # Older releases mount proc in privileged_op with arg1; newer releases
    # mount it directly with dest_path. Match the shared mount options only.
    substituteInPlace bubblewrap.c --replace-fail \
      '"proc", MS_NOSUID | MS_NOEXEC | MS_NODEV, NULL)' \
      '"proc", MS_NOSUID | MS_NOEXEC | MS_NODEV, "subset=pid")'
  '';
  meta = (old.meta or { }) // {
    homepage = "https://github.com/burnskp/cloister";
    mainProgram = "bwrap";
  };
})
