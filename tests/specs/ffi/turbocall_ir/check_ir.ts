const dir = import.meta.dirname!;

const { stderr } = await new Deno.Command(Deno.execPath(), {
  args: [
    "run",
    "--log-level=trace",
    "--unstable-ffi",
    "--allow-ffi",
    `${dir}/trigger.ts`,
  ],
  stdout: "null",
  stderr: "piped",
}).output();

const raw = new TextDecoder().decode(stderr);
const ADDR = /0x[0-9a-fA-F][0-9a-fA-F_]*/g;
const blocks: string[] = [];
const lines = raw.split("\n");
let i = 0;
while (i < lines.length) {
  const line = lines[i];
  if (
    line.includes("deno_ffi::turbocall") &&
    line.toLowerCase().includes("turbocall ir:")
  ) {
    const m = line.match(/- ((?:Slow )?[Tt]urbocall IR):/);
    const label = m ? m[1] : "IR";
    const ir: string[] = [`; ${label}`];
    i++;
    while (i < lines.length) {
      const l = lines[i];
      if (/^(DEBUG|TRACE|WARN|ERROR) RS -/.test(l)) break;
      if (l.trim() === "" && ir.at(-1)?.trim() === "}") break;
      ir.push(l);
      i++;
    }
    blocks.push(ir.map((l) => l.replace(ADDR, "<addr>")).join("\n"));
  } else {
    i++;
  }
}

blocks.sort((a, b) => {
  const fa = a.match(/function %(\S+)/)?.[1] ?? a;
  const fb = b.match(/function %(\S+)/)?.[1] ?? b;
  return fa.localeCompare(fb);
});

console.log(blocks.join("\n\n"));
