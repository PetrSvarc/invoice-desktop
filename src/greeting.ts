export function dashboardGreeting(name: string, at: Date = new Date()): string {
  const hour = at.getHours();
  const salutation = hour >= 5 && hour < 10
    ? 'Dobré ráno'
    : hour >= 10 && hour < 18
      ? 'Dobrý den'
      : 'Dobrý večer';
  const savedName = name.trim();
  return savedName ? `${salutation}, ${savedName}` : salutation;
}
