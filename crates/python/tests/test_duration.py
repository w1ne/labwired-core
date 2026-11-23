"""Exact Python duration parsing, independent of firmware execution."""
from decimal import Decimal, Inexact, Rounded, ROUND_DOWN, localcontext

import pytest

from labwired import _nanoseconds


@pytest.mark.parametrize('value, expected', [
    ('1s', 1_000_000_000),
    ('1ms', 1_000_000),
    ('1us', 1_000),
    ('1ns', 1),
    ('  .0000000001 s ', 1),
    ('1.0000000001ns', 2),
    ('1.000000000000000000000000000001ns', 2),
    ('.999999999999999999999999999999ns', 1),
    ('18446744073709551615ns', 2**64 - 1),
    ('18446744073.709551615s', 2**64 - 1),
    (Decimal('18446744073.7095516140000000001'), 2**64 - 1),
    (Decimal('1e-1000000'), 1),
    (Decimal('0e1000000'), 0),
    (Decimal('-0e-1000000'), 0),
    (0, 0),
    (0.001, 1_000_000),
    (Decimal('0.0010000000000000000000000000001'), 1_000_001),
])
@pytest.mark.parametrize('precision', [2, 6, 28, 80])
def test_exact_nanosecond_ceiling_ignores_decimal_context(value, expected, precision):
    with localcontext() as context:
        context.prec = precision
        context.rounding = ROUND_DOWN
        context.Emax = 9
        context.Emin = -9
        context.traps[Inexact] = True
        context.traps[Rounded] = True
        context.clear_flags()
        assert _nanoseconds(value) == expected
        assert not any(context.flags.values())


@pytest.mark.parametrize('value', [
    '18446744073709551615.0000000000000000001ns',
    '18446744073.7095516150000000000000001s',
    '18446744073709551616ns',
    Decimal('1e1000000'),
    2**64,
])
def test_out_of_range_duration(value):
    with localcontext() as context:
        context.prec = 2
        context.traps[Inexact] = True
        with pytest.raises(ValueError, match='duration exceeds the supported range'):
            _nanoseconds(value)


@pytest.mark.parametrize('value', [
    -1, Decimal('-0.0000000001'), float('inf'), float('-inf'),
    float('nan'), Decimal('sNaN'),
])
def test_nonfinite_or_negative_duration(value):
    with pytest.raises(ValueError, match='duration must be finite and nonnegative'):
        _nanoseconds(value)


@pytest.mark.parametrize('value', [True, False, None, object()])
def test_duration_type_errors(value):
    with pytest.raises(TypeError, match='duration must be'):
        _nanoseconds(value)


@pytest.mark.parametrize('value', ['-1s', 'nan', '1e-3s', '1minute', '', '1.2.3ms'])
def test_duration_string_errors(value):
    with pytest.raises(ValueError, match='duration must use s, ms, us, or ns'):
        _nanoseconds(value)


def test_fraction_with_more_than_integer_conversion_limit_digits():
    # Parsing a legal duration must not convert its entire coefficient to int.
    assert _nanoseconds('1.' + '0' * 5000 + '1ns') == 2
