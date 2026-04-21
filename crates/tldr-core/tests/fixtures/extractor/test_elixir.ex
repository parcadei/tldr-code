# Expected: 3f 1c 0m (3 functions, 1 class/module, 0 methods)
# Adversarial: Elixir defmodule counts as class; def/defp are functions not methods

defmodule Animal do
  def speak(name) do
    "#{name} says ..."
  end

  def create(name) do
    %{name: name, type: :animal}
  end

  defp helper(x) do
    x * 2
  end
end
