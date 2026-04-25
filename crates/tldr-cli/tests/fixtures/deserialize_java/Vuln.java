// Fixture for VAL-002 (issue #11): Java unsafe deserialization sink fed by
// a tainted source. The tldr-core taint scanner needs:
//   - a source pattern from `get_sources(Language::Java)` — we use
//     `request.getParameter(` to taint variable `payload`
//   - propagation: assign `payload` into `stream` so `stream` becomes tainted
//   - a sink pattern from `get_sinks(VulnType::Deserialization, Language::Java)`
//     — we use `ObjectInputStream(` and `readObject(`
//   - the tainted variable name appearing on the SAME line as the sink
//     pattern (the scanner does `line.contains(sink_pattern) && line.contains(var)`)
//
// On unfixed HEAD, the resulting finding is mislabeled as SqlInjection in
// the CLI's local VulnType (the wildcard match arm in
// crates/tldr-cli/src/commands/remaining/vuln.rs:650 maps every
// non-{Sql,Cmd,Xss,Path} variant to SqlInjection). After the fix, the
// finding is correctly labeled as Deserialization.

import java.io.ObjectInputStream;
import java.io.ByteArrayInputStream;
import javax.servlet.http.HttpServletRequest;

public class Vuln {
    public Object readUser(HttpServletRequest request) throws Exception {
        // taint source: request.getParameter assigns to `payload`
        String payload = request.getParameter("data");
        // sink: payload appears on the same line as readObject( and ObjectInputStream(
        Object result = new ObjectInputStream(new ByteArrayInputStream(payload.getBytes())).readObject();
        return result;
    }
}
