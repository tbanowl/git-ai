 const exe = `${process.env.USERPROFILE}\\.git-ai\\bin\\git-proxy.exe`

  const p = Bun.spawn({
    cmd: [exe, "--version"],
    stdout: "pipe",
    stderr: "pipe",
  })

  const [stdout, stderr, code] = await Promise.all([
    new Response(p.stdout).text(),
    new Response(p.stderr).text(),
    p.exited,
  ])

  console.log({ exe, code, stdout, stderr })
