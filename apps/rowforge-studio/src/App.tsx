import { Route, Routes } from "react-router-dom";
import { BootGate } from "./pages/BootGate";
import { ExecDetailPage } from "./pages/ExecDetail";
import { AttemptDetailPage } from "./pages/AttemptDetail";
import { NewExecutionWizardPage } from "./pages/NewExecutionWizard";

export default function App() {
  return (
    <Routes>
      <Route path="/" element={<BootGate />} />
      <Route path="/new" element={<NewExecutionWizardPage />} />
      <Route path="/exec/:id" element={<ExecDetailPage />} />
      <Route path="/exec/:id/attempt/:aid" element={<AttemptDetailPage />} />
      <Route path="*" element={<BootGate />} />
    </Routes>
  );
}
