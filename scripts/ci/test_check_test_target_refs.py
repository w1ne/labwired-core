"""Negative self-tests for check_test_target_refs.py (no cargo needed)."""
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_test_target_refs as c  # noqa: E402


def pkgs(tmp: Path):
    main = tmp / "main.rs"
    main.write_text("mod avr_pwm;\nmod avr_tone;\n")
    return {"labwired-core": {"avr": main, "plain": None}, "labwired-cli": {"cli_t": None}}


def errs(text, tmp):
    return c.check_text("x.yml", text, pkgs(tmp))


def test_cases():
    with tempfile.TemporaryDirectory() as d:
        t = Path(d)
        # accepted
        assert not errs("run: cargo test -p labwired-core --test avr -- avr_pwm::\n", t)
        assert not errs("run: cargo test -p labwired-core --test plain\n", t)
        assert not errs("run: cargo test --test cli_t\n", t)
        assert not errs("run: cargo test -p labwired-core --test avr avr_tone::\n", t)
        assert not errs("run: node --test scripts/x.mjs\n", t)
        # an old per-file target
        assert errs("run: cargo test -p labwired-core --test avr_pwm\n", t)
        # right name, wrong package
        assert errs("run: cargo test -p labwired-cli --test plain\n", t)
        # module missing from the group
        assert errs("run: cargo test -p labwired-core --test avr -- nope::\n", t)
        # filters leaking across two grouped binaries
        t2 = {"labwired-core": {"a": t / "main.rs", "b": t / "main.rs"}}
        assert c.check_text("x", "cargo test -p labwired-core --test a --test b -- avr_pwm::\n", t2)
        # a shell variable that expands to an old target
        txt = "for f in avr_pwm plain; do\n  cargo test -p labwired-core --test \"$f\"\ndone\n"
        assert any("avr_pwm" in e for e in errs(txt, t))
        assert not errs("for f in avr plain; do\n  cargo test -p labwired-core --test \"$f\"\ndone\n", t)
        # unresolvable variable is an error unless the loop is marked dynamic
        assert errs("cargo test -p labwired-core --test \"$x\"\n", t)
        assert not errs("# test-target-refs: dynamic\ncargo test -p labwired-core --test \"$x\"\n", t)
        # variable used as a module filter
        assert errs("m=nope\ncargo test -p labwired-core --test avr -- \"$m::\"\n", t)
        assert not errs("m=avr_pwm\ncargo test -p labwired-core --test avr -- \"$m::\"\n", t)
        # nextest binary() filter
        assert errs("cargo nextest run -E 'binary(avr_pwm)'\n", t)
        assert not errs("cargo nextest run -E 'binary(avr)'\n", t)


def test_repository_is_clean():
    assert c.main([]) == 0
