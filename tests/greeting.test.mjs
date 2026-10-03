import assert from 'node:assert/strict';
import { test } from 'node:test';
import { dashboardGreeting } from '../src/greeting.ts';

const localTime = (hour, minute = 0) => new Date(2026, 9, 2, hour, minute);

test('greeting changes at the local morning, daytime, and evening boundaries', () => {
  assert.equal(dashboardGreeting('Petr Švarc', localTime(4, 59)), 'Dobrý večer, Petr Švarc');
  assert.equal(dashboardGreeting('Petr Švarc', localTime(5)), 'Dobré ráno, Petr Švarc');
  assert.equal(dashboardGreeting('Petr Švarc', localTime(9, 59)), 'Dobré ráno, Petr Švarc');
  assert.equal(dashboardGreeting('Petr Švarc', localTime(10)), 'Dobrý den, Petr Švarc');
  assert.equal(dashboardGreeting('Petr Švarc', localTime(17, 59)), 'Dobrý den, Petr Švarc');
  assert.equal(dashboardGreeting('Petr Švarc', localTime(18)), 'Dobrý večer, Petr Švarc');
});

test('greeting preserves the complete saved name and its punctuation', () => {
  assert.equal(dashboardGreeting('  PhDr. Petr Švarc  ', localTime(14)), 'Dobrý den, PhDr. Petr Švarc');
  assert.equal(dashboardGreeting('  Firma s.r.o.  ', localTime(8)), 'Dobré ráno, Firma s.r.o.');
  assert.equal(dashboardGreeting('   ', localTime(8)), 'Dobré ráno');
});
