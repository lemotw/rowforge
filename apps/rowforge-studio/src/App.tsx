import { Route, Routes } from "react-router-dom";
import { BootGate } from "./pages/BootGate";

export default function App() {
  return (
    <Routes>
      <Route path="*" element={<BootGate />} />
    </Routes>
  );
}
